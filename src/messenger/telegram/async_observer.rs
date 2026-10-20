//! # Session Async Event Observer
//!
//! ## Overview
//! Monitors active Antigravity CLI session transcript logs for asynchronous
//! turn events (such as subagent completion or timer notifications) while the
//! agent is idle, and dispatches new messages to Telegram.
//!
//! ## Search Tags
//! #async-observer, #transcript-watcher, #background-turn

use teloxide::prelude::*;
use std::collections::HashSet;
use std::sync::{Arc, Mutex, LazyLock};
use crate::config::CliConfig;
use crate::cli::antigravity::AntigravityCli;
use crate::cli::antigravity::log_parser::{AntigravityLogParser, ParsedLogDelta};
use crate::session::manager::SessionManager;
use notify::{Watcher, RecursiveMode, EventKind};

static WATCHED_SESSIONS: LazyLock<Arc<Mutex<HashSet<String>>>> = LazyLock::new(|| Arc::new(Mutex::new(HashSet::new())));

struct ObserverCtx {
    bot: Bot,
    chat_id: ChatId,
    thread_id: Option<i32>,
    session_id: String,
    cli: AntigravityCli,
    sessions: Arc<SessionManager>,
    config: CliConfig,
}

pub(crate) fn spawn_session_async_observer(
    bot: Bot,
    msg: &Message,
    session_id: String,
    cli: AntigravityCli,
    sessions: Arc<SessionManager>,
    config: CliConfig,
) {
    let (chat_id, thread_id) = (msg.chat.id, msg.thread_id.map(|t| t.0.0));
    let key = format!("{}:{}:{:?}", session_id, chat_id.0, thread_id);
    {
        let mut lock = WATCHED_SESSIONS.lock().unwrap_or_else(|e| e.into_inner());
        if !lock.insert(key.clone()) { return; }
    }
    let ctx = ObserverCtx {
        bot,
        chat_id,
        thread_id,
        session_id,
        cli,
        sessions,
        config,
    };
    tokio::spawn(async move {
        run_observer_loop(ctx).await;
        WATCHED_SESSIONS.lock().unwrap_or_else(|e| e.into_inner()).remove(&key);
    });
}

async fn handle_async_turn_output(
    ctx: &ObserverCtx,
    msg_id: Option<teloxide::types::MessageId>,
    txt: &str,
) {
    let chunks = super::formatting::split_html_message(&super::formatting::markdown_to_telegram_html(txt), 4000);
    let (ok, err, sent_id) = super::stream::send_chunks_to_telegram(&ctx.bot, ctx.chat_id, msg_id, ctx.thread_id, &chunks).await;
    let _ = super::attachments::send_file_attachments(&ctx.bot, ctx.chat_id, ctx.thread_id, txt, &ctx.config).await;
    super::history::log_telegram_message(
        &ctx.config.working_dir,
        &ctx.session_id,
        (ctx.thread_id.map(|t| t as i64), None),
        "bot",
        sent_id,
        txt,
        (ok, err.as_deref()),
    );
}

fn create_brain_dir_watcher(
    brain_dir: &std::path::Path,
    fs_tx: tokio::sync::mpsc::UnboundedSender<()>,
) -> Option<notify::RecommendedWatcher> {
    if let Ok(mut w) = notify::recommended_watcher(move |res: Result<notify::Event, _>| {
        if let Ok(e) = res
            && matches!(e.kind, EventKind::Modify(_) | EventKind::Create(_)) {
                let _ = fs_tx.send(());
            }
    })
        && w.watch(brain_dir, RecursiveMode::Recursive).is_ok() {
            return Some(w);
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
    ctx: &ObserverCtx,
    delta: ParsedLogDelta,
    active_turn: &mut Option<ActiveAsyncTurn>,
) {
    let topic_id = ctx.thread_id.map(|t| t as i64);
    if let Some(ref txt) = delta.final_content {
        let pub_msg_id = active_turn.as_ref().and_then(|t| t.pub_msg_id);
        if let Some(turn) = active_turn { turn.typing.abort(); }
        let lock_arc = ctx.sessions.lock_pool.get((ctx.chat_id.0, topic_id));
        let _fallback = if active_turn.is_none() {
            Some(lock_arc.lock().await)
        } else { None };
        handle_async_turn_output(ctx, pub_msg_id, txt).await;
        *active_turn = None;
    }

    if let Some(ask) = delta.ask_question {
        if let Some(turn) = active_turn { turn.typing.abort(); }
        let sess_data = crate::session::data::SessionData {
            session_id: Some(ctx.session_id.clone()),
            ..Default::default()
        };
        let _ = super::ask_process::handle_ask_question_event(
            &ctx.bot, ctx.chat_id, ctx.thread_id, ask, &sess_data, &ctx.config, &ctx.cli,
        ).await;
        *active_turn = None;
    }
}

async fn ensure_active_turn(
    ctx: &ObserverCtx,
    active_turn: &mut Option<ActiveAsyncTurn>,
) {
    let topic_id = ctx.thread_id.map(|t| t as i64);
    if active_turn.is_none() {
        let lock = ctx.sessions.lock_pool.get((ctx.chat_id.0, topic_id));
        if let Ok(guard) = lock.try_lock_owned() {
            ctx.cli.sessions.set_async_running(&ctx.session_id, true).await;
            let typing = super::typing::AsyncTypingHandle::new(ctx.bot.clone(), ctx.chat_id, ctx.thread_id);
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
    ctx: &ObserverCtx,
    progress_text: &str,
    active_turn: &mut Option<ActiveAsyncTurn>,
) {
    if let Some(turn) = active_turn {
        let _ = super::stream::handle_text_delta(
            &ctx.bot,
            ctx.chat_id,
            ctx.thread_id,
            progress_text,
            &mut turn.last_text,
            &mut turn.pub_msg_id,
            &mut turn.last_edit,
        ).await;
    }
}

async fn check_and_dispatch_delta(
    ctx: &ObserverCtx,
    transcript_path: &std::path::Path,
    last_size: &mut u64,
    parser: &mut AntigravityLogParser,
    active_turn: &mut Option<ActiveAsyncTurn>,
) {
    let Ok(meta) = std::fs::metadata(transcript_path) else { return; };
    let curr_size = meta.len();
    if curr_size <= *last_size { return; }

    let delta = parser.parse_log_delta_structured(transcript_path, Some(*last_size));
    *last_size = delta.new_size;
    *parser = AntigravityLogParser::new();

    if !delta.has_activity { return; }

    ensure_active_turn(ctx, active_turn).await;

    if delta.final_content.is_some() || delta.ask_question.is_some() {
        ctx.cli.sessions.set_async_running(&ctx.session_id, false).await;
        dispatch_terminal_event(ctx, delta, active_turn).await;
    } else if let Some(ref progress_text) = delta.formatted {
        update_active_turn_progress(ctx, progress_text, active_turn).await;
    }
}

async fn handle_running_state_check(
    cli: &AntigravityCli,
    session_id: &str,
    transcript_path: &std::path::Path,
    was_running: &mut bool,
    last_size: &mut u64,
    parser: &mut AntigravityLogParser,
) -> bool {
    let is_running = cli.sessions.is_running(session_id).await;
    let recent_int = cli.sessions.was_recently_interrupted(session_id, std::time::Duration::from_secs(6)).await;
    if is_running || recent_int {
        *was_running = is_running;
        cli.sessions.set_async_running(session_id, false).await;
        *last_size = std::fs::metadata(transcript_path).map(|m| m.len()).unwrap_or(*last_size);
        *parser = AntigravityLogParser::new();
        return true;
    }

    if *was_running {
        *was_running = false;
        cli.sessions.set_async_running(session_id, false).await;
        *last_size = std::fs::metadata(transcript_path).map(|m| m.len()).unwrap_or(*last_size);
        *parser = AntigravityLogParser::new();
        return true;
    }
    false
}

async fn run_observer_loop(ctx: ObserverCtx) {
    let bd = crate::cli::antigravity::events::agy_state_root(Some(&ctx.cli.build_env()))
        .join("brain")
        .join(&ctx.session_id);
    let tp = bd.join(".system_generated").join("logs").join("transcript_full.jsonl");
    let mut last_size = std::fs::metadata(&tp).map(|m| m.len()).unwrap_or(0);
    let mut parser = AntigravityLogParser::new();
    let (fs_tx, mut fs_rx) = tokio::sync::mpsc::unbounded_channel();
    let watcher = create_brain_dir_watcher(&bd, fs_tx);
    let mut tick = tokio::time::interval(tokio::time::Duration::from_secs(4));
    let (mut was_running, mut active_turn) = (false, None::<ActiveAsyncTurn>);

    loop {
        tokio::select! {
            _ = ctx.cli.sessions.interrupt_notify.notified() => {}
            _ = tick.tick() => {}
            res = fs_rx.recv() => if res.is_none() { break; },
        }
        if !ctx.cli.sessions.is_active(&ctx.session_id).await && !tp.exists() { break; }
        if handle_running_state_check(&ctx.cli, &ctx.session_id, &tp, &mut was_running, &mut last_size, &mut parser).await {
            active_turn = None;
            continue;
        }
        if active_turn.as_ref().is_some_and(|t| t.last_activity.elapsed() > std::time::Duration::from_secs(60)) {
            ctx.cli.sessions.set_async_running(&ctx.session_id, false).await;
            active_turn = None;
        }
        check_and_dispatch_delta(&ctx, &tp, &mut last_size, &mut parser, &mut active_turn).await;
    }
    ctx.cli.sessions.set_async_running(&ctx.session_id, false).await;
    drop(watcher);
}
