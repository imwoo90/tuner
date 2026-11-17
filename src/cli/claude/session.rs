//! # Claude Code Persistent Warm Session Pool (session.rs)
//!
//! ## Overview
//! Manages the lifecycle of persistent warm `claude` processes (`--input-format stream-json`),
//! handling stdin control requests (`interrupt`, `set_model`) and process group termination.
//!
//! ## Search Tags
//! #claude-session, #warm-process-pool, #control-request

use super::resolve_claude_model_effort;
pub(crate) use super::spawner::ClaudeProcessIo;
pub use super::spawner::WarmClaudeSession;
use super::spawner::{model_ctrl, spawn_warm_claude};
use crate::cli::antigravity::AntigravityCli;
use crate::cli::common::resolve_warm_workspace;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use tokio::sync::Mutex;

type TopicKey = (i64, Option<i64>);
static EPHEMERAL_RUNS: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn adjust_ephemeral_runs(inc: bool) {
    if inc { EPHEMERAL_RUNS.fetch_add(1, Ordering::SeqCst); } else { EPHEMERAL_RUNS.fetch_sub(1, Ordering::SeqCst); }
}

#[derive(Default)]
struct ClaudeSessionPool { sessions: Mutex<HashMap<TopicKey, Arc<WarmClaudeSession>>> }

fn global_pool() -> &'static ClaudeSessionPool {
    static POOL: OnceLock<ClaudeSessionPool> = OnceLock::new();
    POOL.get_or_init(ClaudeSessionPool::default)
}

/// Queues user answers for a pending `AskUserQuestion` control request on the topic's warm `claude` session.
pub async fn submit_ask_answers(chat_id: i64, topic_id: Option<i64>, answers: &[String]) {
    if let Some(w) = global_pool().sessions.lock().await.get(&(chat_id, topic_id)).cloned() {
        *w.queued_ask_answers.lock().await = Some(answers.to_vec());
    }
}

/// Reaps dead or 24-hour (`86400s`) idle warm `claude` processes from the pool.
pub async fn cleanup_expired() {
    let mut map = global_pool().sessions.lock().await;
    let mut dead = Vec::new();
    for (&k, w) in map.iter() {
        if !w.running.load(Ordering::SeqCst) && (w.activity.is_expired() || !w.is_alive().await) { dead.push(k); }
    }
    for k in dead { if let Some(old) = map.remove(&k) { old.kill_now(); } }
}

pub(crate) async fn acquire_claude_session(
    cli: &AntigravityCli,
    workspace: PathBuf,
    resume_sid: Option<&str>,
    continue_session: bool,
    answering_ask: bool,
) -> Result<(Arc<WarmClaudeSession>, bool), String> {
    cleanup_expired().await;
    let ws = resolve_warm_workspace(cli, workspace);
    crate::cli::common::trust_peer_workspace("claude", &ws, None);
    crate::workspace::skills_helpers::sync_claude_workspace_bridges(&ws);
    if cli.oneshot {
        return spawn_warm_claude(cli, ws, resume_sid, continue_session).map(|w| (w, true));
    }
    let key = (cli.config.chat_id, cli.config.topic_id);
    if let Some(ex) = global_pool().sessions.lock().await.get(&key).cloned() {
        let cur_s = ex.session_id.lock().await.clone();
        let sid_ok = resume_sid.is_some_and(|r| cur_s.as_deref() == Some(r)) || continue_session;
        let busy = ex.running.load(Ordering::SeqCst)
            || ex.pending_ask.lock().await.is_some()
            || ex.queued_ask_answers.lock().await.is_some();
        if busy && (!sid_ok || ex.workspace != ws) {
            return spawn_warm_claude(cli, ws, resume_sid, continue_session).map(|w| (w, true));
        }
    }
    get_or_spawn_warm_session(cli, ws, resume_sid, continue_session, answering_ask).await.map(|w| (w, false))
}

pub(crate) async fn get_or_spawn_warm_session(
    cli: &AntigravityCli, ws: PathBuf, resume_sid: Option<&str>, continue_session: bool, answering_ask: bool,
) -> Result<Arc<WarmClaudeSession>, String> {
    let (req_m, req_e) = resolve_claude_model_effort(cli.config.model.as_deref(), cli.config.effort.as_deref());
    let (key, mut eff_res) = ((cli.config.chat_id, cli.config.topic_id), resume_sid.map(str::to_string));
    let mut map = global_pool().sessions.lock().await;
    if let Some(ex) = map.get(&key).cloned() {
        let (cur_s, cur_e) = (ex.session_id.lock().await.clone(), ex.effort.lock().await.clone());
        let has_ask = ex.pending_ask.lock().await.is_some();
        let ask = answering_ask && (has_ask || ex.queued_ask_answers.lock().await.is_some());
        let sid_ok = resume_sid.is_some_and(|r| cur_s.as_deref() == Some(r)) || continue_session;
        if (!has_ask || answering_ask) && sid_ok && (ask || req_e == cur_e) && ex.workspace == ws && ex.is_alive().await {
            if !ask && let Some(ref m) = req_m && *ex.model.lock().await != Some(m.clone())
                && ex.write_json_line(&model_ctrl(m)).await.is_ok()
            {
                *ex.model.lock().await = Some(m.clone());
            }
            ex.activity.touch();
            return Ok(ex);
        }
        if sid_ok && eff_res.is_none() { eff_res = cur_s; }
        if let Some(old) = map.remove(&key) { old.kill_now(); }
    }
    let warm = spawn_warm_claude(cli, ws, eff_res.as_deref(), continue_session)?;
    map.insert(key, warm.clone());
    Ok(warm)
}

/// Sends a soft JSONL `interrupt` control request (or cancels a pending ask) for a topic.
pub async fn interrupt_session(chat_id: i64, topic_id: Option<i64>) -> bool {
    let Some(w) = global_pool().sessions.lock().await.get(&(chat_id, topic_id)).cloned() else { return false; };
    if w.pending_ask.lock().await.is_some() {
        w.interrupted.store(true, Ordering::SeqCst);
        w.interrupt_notify.notify_one();
        return terminate_topic(chat_id, topic_id, None).await;
    }
    if !w.running.load(Ordering::SeqCst) {
        return false;
    }
    let ctrl = serde_json::json!({ "type": "control_request", "request_id": format!("stop-{}", chat_id), "request": { "subtype": "interrupt" } });
    let _ = w.write_json_line(&ctrl).await;
    w.interrupted.store(true, Ordering::SeqCst);
    w.interrupt_notify.notify_one();
    true
}

async fn find_warm(
    map: &HashMap<TopicKey, Arc<WarmClaudeSession>>,
    chat_id: i64,
    topic_id: Option<i64>,
    session_id: Option<&str>,
) -> Option<Arc<WarmClaudeSession>> {
    if (chat_id != 0 || topic_id.is_some())
        && let Some(w) = map.get(&(chat_id, topic_id)).cloned()
    {
        return Some(w);
    }
    let sid = session_id.filter(|s| !s.is_empty())?;
    for w in map.values() {
        if w.session_id.lock().await.as_deref() == Some(sid) { return Some(w.clone()); }
    }
    None
}

/// Dynamically switches the active model of a warm `claude` session via `set_model` control_request.
pub async fn set_model_live(chat_id: i64, topic_id: Option<i64>, session_id: Option<&str>, model: &str) -> bool {
    let (Some(canonical), _) = resolve_claude_model_effort(Some(model), None) else { return false; };
    let warm = { let map = global_pool().sessions.lock().await; find_warm(&map, chat_id, topic_id, session_id).await };
    let Some(w) = warm else { return false; };
    if w.running.load(Ordering::SeqCst) || w.pending_ask.lock().await.is_some() || w.queued_ask_answers.lock().await.is_some() {
        return false;
    }
    if !super::supports_claude_effort(&canonical) && w.effort.lock().await.is_some() {
        return terminate_topic(chat_id, topic_id, session_id).await;
    }
    let ok = w.write_json_line(&model_ctrl(&canonical)).await.is_ok();
    if ok { *w.model.lock().await = Some(canonical); }
    ok
}

/// Terminates and removes the warm `claude` process for a specific topic or session ID.
pub async fn terminate_topic(chat_id: i64, topic_id: Option<i64>, session_id: Option<&str>) -> bool {
    let (mut map, mut rem) = (global_pool().sessions.lock().await, false);
    if let Some(w) = map.remove(&(chat_id, topic_id)) { w.kill_now(); rem = true; }
    if let Some(sid) = session_id.filter(|s| !s.is_empty()) {
        let mut keys = Vec::new();
        for (k, w) in map.iter() {
            if w.session_id.lock().await.as_deref() == Some(sid) { keys.push(*k); }
        }
        for k in keys { if let Some(w) = map.remove(&k) { w.kill_now(); rem = true; } }
    }
    rem
}

/// Terminates all warm `claude` processes across all topics for a given `chat_id` (or all chats when `chat_id == 0`).
pub async fn terminate_all_for_chat(chat_id: i64) {
    let mut map = global_pool().sessions.lock().await;
    for k in map.keys().filter(|(cid, _)| chat_id == 0 || *cid == chat_id).copied().collect::<Vec<_>>() {
        if let Some(w) = map.remove(&k) { w.kill_now(); }
    }
}

/// Terminates all warm `claude` processes across all topics.
pub async fn terminate_all() { terminate_all_for_chat(0).await; }

/// Returns the OS PID of the active warm `claude` process for `(chat_id, topic_id)`, if any.
pub async fn active_pid(chat_id: i64, topic_id: Option<i64>) -> Option<u32> {
    let warm = global_pool().sessions.lock().await.get(&(chat_id, topic_id)).cloned()?;
    warm.is_alive().await.then_some(warm.pid)
}

/// Returns the live session identifier of the topic's warm `claude` process, if any.
pub async fn active_session_id(chat_id: i64, topic_id: Option<i64>) -> Option<String> {
    let warm = global_pool().sessions.lock().await.get(&(chat_id, topic_id)).cloned()?;
    if !warm.is_alive().await { return None; }
    warm.session_id.lock().await.clone().filter(|s| !s.is_empty())
}

/// Returns `true` if a turn is currently running on the warm `claude` session for the topic or `session_id`.
pub async fn is_running(chat_id: i64, topic_id: Option<i64>, session_id: Option<&str>) -> bool {
    let map = global_pool().sessions.lock().await;
    if chat_id == 0 && topic_id.is_none() && session_id.is_none() {
        return EPHEMERAL_RUNS.load(Ordering::SeqCst) > 0 || map.values().any(|w| w.running.load(Ordering::SeqCst));
    }
    find_warm(&map, chat_id, topic_id, session_id).await.is_some_and(|w| w.running.load(Ordering::SeqCst))
}

/// Returns `true` if a live non-expired warm `claude` session is awaiting `AskUserQuestion` input.
pub async fn has_pending_ask(chat_id: i64, topic_id: Option<i64>, session_id: Option<&str>) -> bool {
    let warm = {
        let map = global_pool().sessions.lock().await;
        if chat_id != 0 || topic_id.is_some() {
            map.get(&(chat_id, topic_id)).cloned()
        } else {
            find_warm(&map, 0, None, session_id).await
        }
    };
    let Some(w) = warm else { return false; };
    !w.activity.is_expired() && w.is_alive().await && w.pending_ask.lock().await.is_some()
}

/// Returns the number of warm `claude` processes currently registered in the pool.
pub async fn active_count() -> usize {
    global_pool().sessions.lock().await.len()
}
