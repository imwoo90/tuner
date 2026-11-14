//! # OpenAI Codex Persistent Warm Session Pool (session.rs)
//!
//! ## Overview
//! Manages persistent warm `codex app-server --stdio` processes per `(chat_id, topic_id)`,
//! performing JSON-RPC `initialize` + `thread/start` handshakes and `turn/interrupt` signaling.
//!
//! ## Search Tags
//! #codex-session, #app-server-stdio, #warm-process-pool, #json-rpc

use super::spawner::spawn_warm_codex;
use crate::cli::antigravity::AntigravityCli;
use crate::cli::common::{kill_process_group, resolve_warm_workspace, write_json_stdin, WarmActivityTracker};
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use tokio::io::{BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio::sync::{Mutex, Notify};

type TopicKey = (i64, Option<i64>);
static EPHEMERAL_RUNS: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn adjust_ephemeral_runs(inc: bool) {
    if inc { EPHEMERAL_RUNS.fetch_add(1, Ordering::SeqCst); } else { EPHEMERAL_RUNS.fetch_sub(1, Ordering::SeqCst); }
}

pub(crate) struct CodexProcessIo {
    pub(crate) child: Child,
    pub(crate) stdout: Lines<BufReader<ChildStdout>>,
}

/// Persistent warm `codex app-server --stdio` session bound to a conversation topic.
pub struct WarmCodexSession {
    /// OS process ID of the warm `codex app-server` child.
    pub pid: u32,
    /// Active Codex thread UUID.
    pub thread_id: String,
    /// Workspace path where the warm process was started.
    pub workspace: PathBuf,
    /// 24-hour activity tracker matching Antigravity PTY session expiration.
    pub activity: WarmActivityTracker,
    pub(crate) pending_ask: Mutex<Option<(serde_json::Value, Vec<String>)>>,
    pub(crate) queued_ask_answers: Mutex<Option<Vec<String>>>,
    pub(crate) active_turn_id: Mutex<Option<String>>,
    pub(crate) next_req_id: AtomicU64,
    pub(crate) stdin: Mutex<ChildStdin>,
    pub(crate) io: Mutex<CodexProcessIo>,
    pub(crate) running: AtomicBool,
    pub(crate) interrupted: AtomicBool,
    pub(crate) interrupt_notify: Notify,
}

impl WarmCodexSession {
    pub(crate) async fn is_alive(&self) -> bool {
        self.io.try_lock().map(|mut io| io.child.try_wait().map(|s| s.is_none()).unwrap_or(false)).unwrap_or(true)
    }

    pub(crate) async fn write_rpc(&self, payload: &serde_json::Value) -> Result<(), String> {
        self.activity.touch();
        write_json_stdin(&mut *self.stdin.lock().await, payload).await
    }

    pub(crate) async fn send_turn_interrupt(&self, turn_id: &str) {
        let id = self.next_req_id.fetch_add(1, Ordering::SeqCst);
        let _ = self.write_rpc(&json!({
            "jsonrpc": "2.0", "id": id, "method": "turn/interrupt",
            "params": { "threadId": self.thread_id, "turnId": turn_id }
        })).await;
    }

    pub(crate) fn kill_now(&self) {
        if self.pid > 0 { kill_process_group(self.pid); }
    }
}

#[derive(Default)]
struct CodexSessionPool { sessions: Mutex<HashMap<TopicKey, Arc<WarmCodexSession>>> }

fn global_pool() -> &'static CodexSessionPool {
    static POOL: OnceLock<CodexSessionPool> = OnceLock::new();
    POOL.get_or_init(CodexSessionPool::default)
}

/// Queues user answers for a pending `item/tool/requestUserInput` request on the topic's warm `codex` session.
pub async fn submit_ask_answers(chat_id: i64, topic_id: Option<i64>, answers: &[String]) {
    if let Some(w) = global_pool().sessions.lock().await.get(&(chat_id, topic_id)).cloned() {
        *w.queued_ask_answers.lock().await = Some(answers.to_vec());
    }
}

/// Reaps dead or 24-hour (`86400s`) idle warm `codex` processes from the pool.
pub async fn cleanup_expired() {
    let mut map = global_pool().sessions.lock().await;
    let mut dead = Vec::new();
    for (&k, w) in map.iter() {
        if !w.running.load(Ordering::SeqCst) && (w.activity.is_expired() || !w.is_alive().await) { dead.push(k); }
    }
    for k in dead { if let Some(old) = map.remove(&k) { old.kill_now(); } }
}

pub(crate) async fn acquire_codex_session(
    cli: &AntigravityCli,
    workspace: PathBuf,
    resume_sid: Option<&str>,
    continue_session: bool,
    answering_ask: bool,
) -> Result<(Arc<WarmCodexSession>, bool), String> {
    cleanup_expired().await;
    let ws = resolve_warm_workspace(cli, workspace);
    if cli.oneshot {
        return spawn_warm_codex(cli, ws, resume_sid).await.map(|w| (w, true));
    }
    let key = (cli.config.chat_id, cli.config.topic_id);
    if let Some(ex) = global_pool().sessions.lock().await.get(&key).cloned() {
        let sid_ok = resume_sid.is_some_and(|r| ex.thread_id == r) || continue_session;
        let busy = ex.running.load(Ordering::SeqCst)
            || ex.pending_ask.lock().await.is_some()
            || ex.queued_ask_answers.lock().await.is_some();
        if busy && (!sid_ok || ex.workspace != ws) {
            return spawn_warm_codex(cli, ws, resume_sid).await.map(|w| (w, true));
        }
    }
    get_or_spawn_warm_codex(cli, ws, resume_sid, continue_session, answering_ask).await.map(|w| (w, false))
}

pub(crate) async fn get_or_spawn_warm_codex(
    cli: &AntigravityCli,
    ws: PathBuf,
    resume_sid: Option<&str>,
    continue_session: bool,
    answering_ask: bool,
) -> Result<Arc<WarmCodexSession>, String> {
    let key = (cli.config.chat_id, cli.config.topic_id);
    if let Some(ex) = global_pool().sessions.lock().await.get(&key).cloned() {
        let has_ask = ex.pending_ask.lock().await.is_some();
        if (!has_ask || answering_ask)
            && (resume_sid.is_some_and(|r| ex.thread_id == r) || continue_session)
            && ex.workspace == ws
            && ex.is_alive().await
        {
            ex.activity.touch();
            return Ok(ex);
        }
        if let Some(old) = global_pool().sessions.lock().await.remove(&key) { old.kill_now(); }
    }
    let warm = spawn_warm_codex(cli, ws, resume_sid).await?;
    if let Some(old) = global_pool().sessions.lock().await.insert(key, warm.clone()) { old.kill_now(); }
    Ok(warm)
}

/// Sends a JSON-RPC `turn/interrupt` request (or cancels a pending ask) for a topic.
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
    if let Some(tid) = w.active_turn_id.lock().await.clone() { w.send_turn_interrupt(&tid).await; }
    w.interrupted.store(true, Ordering::SeqCst);
    w.interrupt_notify.notify_one();
    true
}

/// Terminates and removes the warm `codex` process for a specific topic or thread ID.
pub async fn terminate_topic(chat_id: i64, topic_id: Option<i64>, session_id: Option<&str>) -> bool {
    let (mut map, mut rem) = (global_pool().sessions.lock().await, false);
    if let Some(w) = map.remove(&(chat_id, topic_id)) { w.kill_now(); rem = true; }
    if let Some(sid) = session_id.filter(|s| !s.is_empty()) {
        for k in map.iter().filter(|(_, w)| w.thread_id == sid).map(|(k, _)| *k).collect::<Vec<_>>() {
            if let Some(w) = map.remove(&k) { w.kill_now(); rem = true; }
        }
    }
    rem
}

/// Terminates all warm `codex` processes across all topics for a given `chat_id` (or all chats when `chat_id == 0`).
pub async fn terminate_all_for_chat(chat_id: i64) {
    let mut map = global_pool().sessions.lock().await;
    for k in map.keys().filter(|(cid, _)| chat_id == 0 || *cid == chat_id).copied().collect::<Vec<_>>() {
        if let Some(w) = map.remove(&k) { w.kill_now(); }
    }
}

/// Terminates all warm `codex` processes across all topics.
pub async fn terminate_all() { terminate_all_for_chat(0).await; }

/// Returns the OS PID of the active warm `codex` process for `(chat_id, topic_id)`, if any.
pub async fn active_pid(chat_id: i64, topic_id: Option<i64>) -> Option<u32> {
    let warm = global_pool().sessions.lock().await.get(&(chat_id, topic_id)).cloned()?;
    warm.is_alive().await.then_some(warm.pid)
}

/// Returns the live `thread_id` of the topic's warm `codex` process, if any.
pub async fn active_session_id(chat_id: i64, topic_id: Option<i64>) -> Option<String> {
    let warm = global_pool().sessions.lock().await.get(&(chat_id, topic_id)).cloned()?;
    warm.is_alive().await.then(|| warm.thread_id.clone()).filter(|s| !s.is_empty())
}

/// Returns `true` if a turn is currently running on the warm `codex` session for the topic or `session_id`.
pub async fn is_running(chat_id: i64, topic_id: Option<i64>, session_id: Option<&str>) -> bool {
    let map = global_pool().sessions.lock().await;
    if chat_id == 0 && topic_id.is_none() && session_id.is_none() {
        return EPHEMERAL_RUNS.load(Ordering::SeqCst) > 0 || map.values().any(|w| w.running.load(Ordering::SeqCst));
    }
    if let Some(w) = map.get(&(chat_id, topic_id)) { return w.running.load(Ordering::SeqCst); }
    session_id.filter(|s| !s.is_empty()).is_some_and(|sid| {
        map.values().any(|w| w.thread_id == sid && w.running.load(Ordering::SeqCst))
    })
}

/// Returns `true` if a live non-expired warm `codex` session is awaiting `requestUserInput`.
pub async fn has_pending_ask(chat_id: i64, topic_id: Option<i64>, session_id: Option<&str>) -> bool {
    let warm = {
        let map = global_pool().sessions.lock().await;
        if chat_id != 0 { map.get(&(chat_id, topic_id)).cloned() } else {
            session_id.filter(|s| !s.is_empty()).and_then(|sid| map.values().find(|w| w.thread_id == sid).cloned())
        }
    };
    let Some(w) = warm else { return false; };
    !w.activity.is_expired() && w.is_alive().await && w.pending_ask.lock().await.is_some()
}

/// Returns the number of warm `codex` processes currently registered in the pool.
pub async fn active_count() -> usize { global_pool().sessions.lock().await.len() }
