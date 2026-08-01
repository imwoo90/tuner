//! # Session Async Event Observer
//!
//! Monitors active Antigravity CLI session transcript logs for asynchronous
//! turn events (such as subagent completion or timer notifications) while the
//! agent is idle, and dispatches new messages to Telegram.

use teloxide::prelude::*;
use teloxide::types::ChatAction;
use std::collections::HashSet;
use std::sync::{Arc, Mutex, LazyLock};
use crate::config::CliConfig;
use crate::cli::antigravity::AntigravityCli;
use crate::session::manager::SessionManager;
use notify::{Watcher, RecursiveMode, EventKind};

static WATCHED_SESSIONS: LazyLock<Arc<Mutex<HashSet<String>>>> = LazyLock::new(|| Arc::new(Mutex::new(HashSet::new())));

pub(crate) fn spawn_session_async_observer(
    bot: Bot,
    msg: &Message,
    session_id: String,
    cli: AntigravityCli,
    sessions: Arc<SessionManager>,
    config: CliConfig,
) {
    let mut lock = WATCHED_SESSIONS.lock().unwrap();
    if lock.contains(&session_id) {
        return;
    }
    lock.insert(session_id.clone());
    drop(lock);

    let chat_id = msg.chat.id;
    let thread_id = msg.thread_id;

    tokio::spawn(async move {
        run_observer_loop(bot, chat_id, thread_id, session_id.clone(), cli, sessions, config).await;
        let mut lock = WATCHED_SESSIONS.lock().unwrap();
        lock.remove(&session_id);
    });
}

async fn handle_async_turn_output(
    bot: &Bot,
    chat_id: ChatId,
    thread_id: Option<i32>,
    session_id: &str,
    config: &CliConfig,
    txt: &str,
) {
    let mut action_req = bot.send_chat_action(chat_id, ChatAction::Typing);
    if let Some(t) = thread_id { action_req = action_req.message_thread_id(t); }
    let _ = action_req.await;

    let html_text = super::formatting::markdown_to_telegram_html(txt);
    let chunks = super::formatting::split_html_message(&html_text, 4000);
    for chunk in &chunks {
        let mut msg_req = bot.send_message(chat_id, chunk)
            .parse_mode(teloxide::types::ParseMode::Html);
        if let Some(t) = thread_id { msg_req = msg_req.message_thread_id(t); }
        if let Ok(sent) = msg_req.await {
            super::history::log_telegram_message(
                &config.working_dir,
                session_id,
                "bot",
                Some(sent.id.0),
                txt,
                true,
                None,
            );
        }
    }
}

fn create_brain_dir_watcher(
    brain_dir: &std::path::Path,
    fs_tx: tokio::sync::mpsc::UnboundedSender<()>,
) -> Option<notify::RecommendedWatcher> {
    if let Ok(mut w) = notify::recommended_watcher(move |res: Result<notify::Event, _>| {
        if let Ok(e) = res {
            if matches!(e.kind, EventKind::Modify(_) | EventKind::Create(_)) {
                let _ = fs_tx.send(());
            }
        }
    }) {
        if w.watch(brain_dir, RecursiveMode::Recursive).is_ok() {
            return Some(w);
        }
    }
    None
}

async fn check_and_dispatch_delta(
    bot: &Bot,
    chat_id: ChatId,
    thread_id: Option<i32>,
    session_id: &str,
    cli: &AntigravityCli,
    config: &CliConfig,
    transcript_path: &std::path::Path,
    last_size: &mut u64,
    parser: &mut crate::cli::antigravity::log_parser::AntigravityLogParser,
) {
    if let Ok(meta) = std::fs::metadata(transcript_path) {
        let curr_size = meta.len();
        if curr_size > *last_size {
            let (new_size, formatted_txt, ask_data) = parser.parse_log_delta(transcript_path, Some(*last_size));
            *last_size = new_size;
            *parser = crate::cli::antigravity::log_parser::AntigravityLogParser::new();

            if let Some(ref txt) = formatted_txt {
                handle_async_turn_output(bot, chat_id, thread_id, session_id, config, txt).await;
            }

            if let Some(ask) = ask_data {
                let sess_data = crate::session::data::SessionData {
                    session_id: Some(session_id.to_string()),
                    ..Default::default()
                };
                let _ = super::ask_process::handle_ask_question_event(bot, chat_id, thread_id, ask, &sess_data, config, cli).await;
            }
        }
    }
}

async fn handle_running_state_check(
    cli: &AntigravityCli,
    session_id: &str,
    transcript_path: &std::path::Path,
    was_running: &mut bool,
    last_size: &mut u64,
    parser: &mut crate::cli::antigravity::log_parser::AntigravityLogParser,
) -> bool {
    let is_running = cli.sessions.is_running(session_id).await;
    if is_running {
        *was_running = true;
        *last_size = std::fs::metadata(transcript_path).map(|m| m.len()).unwrap_or(*last_size);
        *parser = crate::cli::antigravity::log_parser::AntigravityLogParser::new();
        return true;
    }

    if *was_running {
        *was_running = false;
        *last_size = std::fs::metadata(transcript_path).map(|m| m.len()).unwrap_or(*last_size);
        *parser = crate::cli::antigravity::log_parser::AntigravityLogParser::new();
        return true;
    }
    false
}

async fn run_observer_loop(
    bot: Bot,
    chat_id: ChatId,
    thread_id: Option<i32>,
    session_id: String,
    cli: AntigravityCli,
    _sessions: Arc<SessionManager>,
    config: CliConfig,
) {
    let env = cli.build_env();
    let brain_dir = crate::cli::antigravity::events::agy_state_root(Some(&env))
        .join("brain")
        .join(&session_id);
    let transcript_path = brain_dir.join(".system_generated").join("logs").join("transcript_full.jsonl");

    let mut last_size = std::fs::metadata(&transcript_path).map(|m| m.len()).unwrap_or(0);
    let mut parser = crate::cli::antigravity::log_parser::AntigravityLogParser::new();

    let (fs_tx, mut fs_rx) = tokio::sync::mpsc::unbounded_channel();
    let watcher = create_brain_dir_watcher(&brain_dir, fs_tx);

    let mut fallback = tokio::time::interval(tokio::time::Duration::from_secs(4));

    let mut was_running = false;
    loop {
        tokio::select! {
            _ = fallback.tick() => {}
            res = fs_rx.recv() => {
                if res.is_none() {
                    break;
                }
            }
        }

        if !cli.sessions.is_active(&session_id).await && !transcript_path.exists() {
            break;
        }

        if handle_running_state_check(&cli, &session_id, &transcript_path, &mut was_running, &mut last_size, &mut parser).await {
            continue;
        }

        check_and_dispatch_delta(&bot, chat_id, thread_id, &session_id, &cli, &config, &transcript_path, &mut last_size, &mut parser).await;
    }
    drop(watcher);
}
