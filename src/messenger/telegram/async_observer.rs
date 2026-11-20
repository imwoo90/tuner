//! # Session Async Event Observer
//!
//! ## Overview
//! Monitors active Antigravity CLI session transcript logs for asynchronous
//! turn events (subagents, timers, background asks) while idle, dispatching
//! updates to Telegram and/or the Web Dashboard event hub.
//!
//! ## Search Tags
//! #async-observer, #transcript-watcher, #background-turn

use teloxide::prelude::*;
use teloxide::types::MessageId;
use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Mutex, LazyLock};
use std::time::{Duration, Instant};
use crate::config::CliConfig;
use crate::cli::antigravity::AntigravityCli;
use crate::cli::antigravity::log_parser::{AntigravityLogParser, ParsedLogDelta};
use crate::messenger::web::events_bridge as wev;
use crate::session::data::SessionData;
use crate::session::key::SessionKey;
use crate::session::manager::SessionManager;
use super::observer_subagents::ObserverSubagentState;
pub(crate) use crate::cli::antigravity::polling::create_brain_dir_watcher;

static WATCHED_SESSIONS: LazyLock<Arc<Mutex<HashSet<String>>>> = LazyLock::new(|| Arc::new(Mutex::new(HashSet::new())));

struct ObserverCtx {
    bot: Option<Bot>,
    transport: String,
    chat_id: ChatId,
    thread_id: Option<i32>,
    session_id: String,
    cli: AntigravityCli,
    sessions: Arc<SessionManager>,
    config: CliConfig,
}

impl ObserverCtx {
    fn key(&self) -> SessionKey {
        SessionKey { transport: self.transport.clone(), chat_id: self.chat_id.0, topic_id: self.thread_id.map(|t| t as i64) }
    }
    fn tid(&self) -> Option<i64> {
        self.thread_id.map(|t| t as i64)
    }
}

pub(crate) fn spawn_session_async_observer_target(
    bot: Option<Bot>,
    target: (i64, Option<i64>, String),
    session_id: String,
    cli: AntigravityCli,
    sessions: Arc<SessionManager>,
    config: CliConfig,
) {
    let (cid, tid, tr) = target;
    let key = format!("{tr}:{session_id}:{cid}:{tid:?}");
    if !WATCHED_SESSIONS.lock().unwrap_or_else(|e| e.into_inner()).insert(key.clone()) { return; }
    let bot = bot.filter(|_| tr == "tg" && cid != 0);
    let ctx = ObserverCtx { bot, transport: tr, chat_id: ChatId(cid), thread_id: tid.map(|t| t as i32), session_id, cli, sessions, config };
    tokio::spawn(async move {
        run_observer_loop(ctx).await;
        WATCHED_SESSIONS.lock().unwrap_or_else(|e| e.into_inner()).remove(&key);
    });
}

pub(crate) fn spawn_session_async_observer(
    bot: Bot,
    msg: &Message,
    session_id: String,
    cli: AntigravityCli,
    sessions: Arc<SessionManager>,
    config: CliConfig,
) {
    spawn_session_async_observer_target(
        Some(bot), (msg.chat.id.0, msg.thread_id.map(|t| t.0.0 as i64), "tg".into()), session_id, cli, sessions, config,
    );
}

async fn get_active_session(ctx: &ObserverCtx) -> Option<SessionData> {
    ctx.sessions.get_active(&ctx.key()).await.ok().flatten()
}

pub(crate) use super::stream::{build_observer_ask_session, persist_async_antigravity_turn};

async fn handle_async_turn_output(ctx: &ObserverCtx, msg_id: Option<MessageId>, txt: &str) {
    let (tid64, key) = (ctx.tid(), ctx.key());
    let (ok, err, sent_id) = if let Some(ref bot) = ctx.bot {
        let chunks = super::formatting::split_html_message(&super::formatting::markdown_to_telegram_html(txt), 4000);
        let res = super::stream::send_chunks_to_telegram(bot, ctx.chat_id, msg_id, ctx.thread_id, &chunks).await;
        let _ = super::attachments::send_file_attachments(bot, ctx.chat_id, ctx.thread_id, txt, &ctx.config).await;
        res
    } else {
        (true, None, None)
    };
    let tname = persist_async_antigravity_turn(&ctx.sessions, &key, &ctx.session_id, &ctx.config.provider).await;
    super::history::log_telegram_message(&ctx.config.working_dir, &ctx.session_id, (tid64, tname.as_deref()), "bot", sent_id, txt, (ok, err.as_deref()));
    wev::emit_turn_res(&ctx.transport, ctx.chat_id.0, tid64, Some(ctx.session_id.clone()), txt, !ok).await;
}

struct ActiveAsyncTurn {
    _guard: tokio::sync::OwnedMutexGuard<()>,
    typing: Option<super::typing::AsyncTypingHandle>,
    last_activity: Instant,
    pub_msg_id: Option<MessageId>,
    last_edit: Instant,
    last_text: String,
}

async fn dispatch_terminal_event(ctx: &ObserverCtx, delta: ParsedLogDelta, active_turn: &mut Option<ActiveAsyncTurn>) {
    let tid64 = ctx.tid();
    if let Some(ref txt) = delta.final_content {
        let pub_msg_id = active_turn.as_ref().and_then(|t| t.pub_msg_id);
        if let Some(t) = active_turn.as_ref().and_then(|x| x.typing.as_ref()) { t.abort(); }
        let lock_arc = ctx.sessions.lock_pool.get((ctx.chat_id.0, tid64));
        let _fb = if active_turn.is_none() { Some(lock_arc.lock().await) } else { None };
        handle_async_turn_output(ctx, pub_msg_id, txt).await;
        *active_turn = None;
    }
    if let Some(ask) = delta.ask_question {
        if let Some(t) = active_turn.as_ref().and_then(|x| x.typing.as_ref()) { t.abort(); }
        if let Some(ref bot) = ctx.bot {
            let mut sd = build_observer_ask_session(get_active_session(ctx).await, ctx.chat_id.0, tid64, &ctx.session_id);
            sd.transport = ctx.transport.clone();
            let _ = super::ask_process::handle_ask_question_event(bot, ctx.chat_id, ctx.thread_id, ask, &sd, (&ctx.config, &ctx.cli, &ctx.sessions)).await;
        } else {
            wev::web_obs_ask(&ctx.transport, ctx.chat_id.0, tid64, &ctx.session_id, ask).await;
        }
        *active_turn = None;
    }
}

async fn ensure_active_turn(
    ctx: &ObserverCtx,
    active_turn: &mut Option<ActiveAsyncTurn>,
    sub_state: &mut ObserverSubagentState,
) {
    if active_turn.is_none() {
        let lock = ctx.sessions.lock_pool.get((ctx.chat_id.0, ctx.tid()));
        if let Ok(guard) = lock.try_lock_owned() {
            ctx.cli.sessions.set_async_running(&ctx.session_id, true).await;
            let typing = ctx.bot.as_ref().map(|b| super::typing::AsyncTypingHandle::new(b.clone(), ctx.chat_id, ctx.thread_id));
            let adopt_id = sub_state.take_adoptable_msg_id();
            *active_turn = Some(ActiveAsyncTurn { _guard: guard, typing, last_activity: Instant::now(), pub_msg_id: adopt_id, last_edit: Instant::now(), last_text: String::new() });
        }
    } else if let Some(turn) = active_turn {
        turn.last_activity = Instant::now();
    }
}

async fn check_and_dispatch_delta(
    ctx: &ObserverCtx,
    transcript_path: &Path,
    last_size: &mut u64,
    parser: &mut AntigravityLogParser,
    active_turn: &mut Option<ActiveAsyncTurn>,
    sub_state: &mut ObserverSubagentState,
) {
    let Ok(meta) = std::fs::metadata(transcript_path) else { return };
    if meta.len() <= *last_size { return; }
    let delta = parser.parse_log_delta_structured(transcript_path, Some(*last_size));
    *last_size = delta.new_size;
    *parser = AntigravityLogParser::new();
    if !delta.has_activity { return; }
    crate::upgrade::record_user_activity();
    ensure_active_turn(ctx, active_turn, sub_state).await;
    if delta.final_content.is_some() || delta.ask_question.is_some() {
        ctx.cli.sessions.set_async_running(&ctx.session_id, false).await;
        dispatch_terminal_event(ctx, delta, active_turn).await;
    } else if let (Some(pt), Some(turn)) = (delta.formatted.as_deref(), active_turn.as_mut()) {
        wev::obs_delta(&ctx.transport, ctx.chat_id.0, ctx.tid(), &ctx.session_id, pt).await;
        if let Some(ref bot) = ctx.bot {
            let _ = super::stream::handle_text_delta(bot, ctx.chat_id, ctx.thread_id, pt, &mut turn.last_text, &mut turn.pub_msg_id, &mut turn.last_edit).await;
        }
    }
}

async fn should_exit_observer(ctx: &ObserverCtx, tp: &Path) -> bool {
    let cur = get_active_session(ctx).await.is_some_and(|s| s.get_session_id("antigravity") == ctx.session_id);
    !ctx.cli.sessions.is_active(&ctx.session_id).await && (!cur || !tp.exists())
}

async fn run_observer_loop(ctx: ObserverCtx) {
    let br = crate::cli::antigravity::events::agy_state_root(Some(&ctx.cli.build_env())).join("brain");
    let tp = br.join(&ctx.session_id).join(".system_generated/logs/transcript_full.jsonl");
    let mut last_size = std::fs::metadata(&tp).map(|m| m.len()).unwrap_or(0);
    let (mut parser, mut sub) = (AntigravityLogParser::new(), ObserverSubagentState::new());
    let (fs_tx, mut fs_rx) = tokio::sync::mpsc::unbounded_channel();
    let watcher = create_brain_dir_watcher(&br.join(&ctx.session_id), fs_tx);
    let (mut fs_closed, mut tick) = (watcher.is_none(), tokio::time::interval(Duration::from_secs(2)));
    let (mut was_running, mut active) = (false, None::<ActiveAsyncTurn>);
    loop {
        tokio::select! {
            _ = ctx.cli.sessions.interrupt_notify.notified() => {}
            _ = ctx.cli.pty_sync.observer_notify.notified() => {}
            _ = tick.tick() => {}
            r = fs_rx.recv(), if !fs_closed => if r.is_none() { fs_closed = true; },
        }
        if should_exit_observer(&ctx, &tp).await { break; }
        if sub.sync_running_state(&ctx.cli, &ctx.session_id, &tp, &mut was_running, &mut last_size, &mut parser).await {
            active = None;
            continue;
        }
        if active.as_ref().is_some_and(|t| t.last_activity.elapsed() > Duration::from_secs(300)) {
            ctx.cli.sessions.set_async_running(&ctx.session_id, false).await;
            active = None;
        }
        check_and_dispatch_delta(&ctx, &tp, &mut last_size, &mut parser, &mut active, &mut sub).await;
        if active.is_none() {
            sub.poll_and_stream(ctx.bot.as_ref(), &ctx.transport, ctx.chat_id, ctx.thread_id, &ctx.session_id, &br).await;
        }
    }
    ctx.cli.sessions.set_async_running(&ctx.session_id, false).await;
    drop(watcher);
}
