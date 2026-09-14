//! # Session Async Event Observer
//!
//! Monitors active Antigravity CLI session transcript logs for asynchronous
//! turn events (such as subagent completion or timer notifications) while the
//! agent is idle, and dispatches new messages to Telegram.

use teloxide::prelude::*;
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
    let chat_id = msg.chat.id;
    let thread_id = msg.thread_id.map(|t| t.0.0);
    let key = format!("{}:{}:{:?}", session_id, chat_id.0, thread_id);
    let mut lock = WATCHED_SESSIONS.lock().unwrap();
    if lock.contains(&key) {
        return;
    }
    lock.insert(key.clone());
    drop(lock);

    tokio::spawn(async move {
        run_observer_loop(bot, chat_id, thread_id, session_id.clone(), cli, sessions, config).await;
        let mut lock = WATCHED_SESSIONS.lock().unwrap();
        lock.remove(&key);
    });
}

async fn handle_async_turn_output(
    bot: &Bot,
    chat_id: ChatId,
    msg_id: Option<teloxide::types::MessageId>,
    thread_id: Option<i32>,
    session_id: &str,
    config: &CliConfig,
    txt: &str,
) {
    let html_text = super::formatting::markdown_to_telegram_html(txt);
    let chunks = super::formatting::split_html_message(&html_text, 4000);
    let topic_id = thread_id.map(|t| t as i64);

    let (final_success, final_error, sent_msg_id) =
        super::stream::send_chunks_to_telegram(bot, chat_id, msg_id, thread_id, &chunks).await;

    let _ = super::attachments::send_file_attachments(bot, chat_id, thread_id, txt, config).await;

    super::history::log_telegram_message(
        &config.working_dir,
        session_id,
        topic_id,
        None,
        "bot",
        sent_msg_id,
        txt,
        final_success,
        final_error.as_deref(),
    );
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

struct ActiveAsyncTurn {
    _guard: tokio::sync::OwnedMutexGuard<()>,
    typing: super::typing::AsyncTypingHandle,
    last_activity: std::time::Instant,
    pub_msg_id: Option<teloxide::types::MessageId>,
    last_edit: std::time::Instant,
    last_text: String,
}

async fn dispatch_terminal_event(
    bot: &Bot,
    chat_id: ChatId,
    thread_id: Option<i32>,
    session_id: &str,
    cli: &AntigravityCli,
    sessions: &SessionManager,
    config: &CliConfig,
    delta: crate::cli::antigravity::log_parser::ParsedLogDelta,
    active_turn: &mut Option<ActiveAsyncTurn>,
) {
    let topic_id = thread_id.map(|t| t as i64);
    if let Some(ref txt) = delta.final_content {
        let pub_msg_id = active_turn.as_ref().and_then(|t| t.pub_msg_id);
        if let Some(turn) = active_turn { turn.typing.abort(); }
        let lock_arc = sessions.lock_pool.get((chat_id.0, topic_id));
        let _fallback = if active_turn.is_none() {
            Some(lock_arc.lock().await)
        } else { None };
        handle_async_turn_output(bot, chat_id, pub_msg_id, thread_id, session_id, config, txt).await;
        *active_turn = None;
    }

    if let Some(ask) = delta.ask_question {
        if let Some(turn) = active_turn { turn.typing.abort(); }
        let sess_data = crate::session::data::SessionData {
            session_id: Some(session_id.to_string()),
            ..Default::default()
        };
        let _ = super::ask_process::handle_ask_question_event(bot, chat_id, thread_id, ask, &sess_data, config, cli).await;
        *active_turn = None;
    }
}

fn ensure_active_turn(
    bot: &Bot,
    chat_id: ChatId,
    thread_id: Option<i32>,
    sessions: &SessionManager,
    active_turn: &mut Option<ActiveAsyncTurn>,
) {
    let topic_id = thread_id.map(|t| t as i64);
    if active_turn.is_none() {
        let lock = sessions.lock_pool.get((chat_id.0, topic_id));
        if let Ok(guard) = lock.try_lock_owned() {
            let typing = super::typing::AsyncTypingHandle::new(bot.clone(), chat_id, thread_id);
            *active_turn = Some(ActiveAsyncTurn {
                _guard: guard,
                typing,
                last_activity: std::time::Instant::now(),
                pub_msg_id: None,
                last_edit: std::time::Instant::now(),
                last_text: String::new(),
            });
        }
    } else if let Some(turn) = active_turn {
        turn.last_activity = std::time::Instant::now();
    }
}

async fn update_active_turn_progress(
    bot: &Bot,
    chat_id: ChatId,
    thread_id: Option<i32>,
    progress_text: &str,
    active_turn: &mut Option<ActiveAsyncTurn>,
) {
    if let Some(turn) = active_turn {
        let _ = super::stream::handle_text_delta(
            bot,
            chat_id,
            thread_id,
            progress_text,
            &mut turn.last_text,
            &mut turn.pub_msg_id,
            &mut turn.last_edit,
        ).await;
    }
}

async fn check_and_dispatch_delta(
    bot: &Bot,
    chat_id: ChatId,
    thread_id: Option<i32>,
    session_id: &str,
    cli: &AntigravityCli,
    sessions: &SessionManager,
    config: &CliConfig,
    transcript_path: &std::path::Path,
    last_size: &mut u64,
    parser: &mut crate::cli::antigravity::log_parser::AntigravityLogParser,
    active_turn: &mut Option<ActiveAsyncTurn>,
) {
    let Ok(meta) = std::fs::metadata(transcript_path) else { return; };
    let curr_size = meta.len();
    if curr_size <= *last_size { return; }

    let delta = parser.parse_log_delta_structured(transcript_path, Some(*last_size));
    *last_size = delta.new_size;
    *parser = crate::cli::antigravity::log_parser::AntigravityLogParser::new();

    if !delta.has_activity { return; }

    ensure_active_turn(bot, chat_id, thread_id, sessions, active_turn);

    if delta.final_content.is_some() || delta.ask_question.is_some() {
        dispatch_terminal_event(bot, chat_id, thread_id, session_id, cli, sessions, config, delta, active_turn).await;
    } else if let Some(ref progress_text) = delta.formatted {
        update_active_turn_progress(bot, chat_id, thread_id, progress_text, active_turn).await;
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
    sessions: Arc<SessionManager>,
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
    let mut active_turn: Option<ActiveAsyncTurn> = None;

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
            active_turn = None;
            continue;
        }

        if let Some(ref turn) = active_turn {
            if turn.last_activity.elapsed() > std::time::Duration::from_secs(60) {
                active_turn = None;
            }
        }

        check_and_dispatch_delta(
            &bot, chat_id, thread_id, &session_id, &cli, &sessions, &config,
            &transcript_path, &mut last_size, &mut parser, &mut active_turn,
        ).await;
    }
    drop(watcher);
}

