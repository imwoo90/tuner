//! # Claude Code Persistent Warm Session Pool (session.rs)
//!
//! ## Overview
//! Manages the lifecycle of persistent warm `claude` processes (`--input-format stream-json`),
//! handling stdin control requests (`interrupt`, `set_model`) and process group termination.
//!
//! ## Search Tags
//! #claude-session, #warm-process-pool, #control-request

use super::{build_warm_command, resolve_claude_model_effort};
use crate::cli::antigravity::AntigravityCli;
use crate::cli::common::{kill_process_group, resolve_warm_workspace, write_json_stdin, WarmActivityTracker};
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use tokio::io::{AsyncBufReadExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::{Mutex, Notify};

type TopicKey = (i64, Option<i64>);

pub(crate) struct ClaudeProcessIo {
    pub(crate) child: Child,
    pub(crate) stdout: Lines<BufReader<ChildStdout>>,
}

/// Persistent warm `claude` process bound to a `(chat_id, topic_id)` conversation.
pub struct WarmClaudeSession {
    /// OS process ID of the warm `claude` child.
    pub pid: u32,
    /// Active Claude session UUID once discovered.
    pub session_id: Mutex<Option<String>>,
    /// Active canonical model on this warm process.
    pub model: Mutex<Option<String>>,
    /// Active reasoning effort level on this warm process.
    pub effort: Mutex<Option<String>>,
    /// Workspace path where the warm process was started.
    pub workspace: PathBuf,
    /// 24-hour activity tracker matching Antigravity PTY session expiration.
    pub activity: WarmActivityTracker,
    pub(crate) pending_ask: Mutex<Option<(String, serde_json::Value)>>,
    pub(crate) queued_ask_answers: Mutex<Option<Vec<String>>>,
    pub(crate) stdin: Mutex<ChildStdin>,
    pub(crate) io: Mutex<ClaudeProcessIo>,
    pub(crate) running: AtomicBool,
    pub(crate) interrupted: AtomicBool,
    pub(crate) interrupt_notify: Notify,
}

impl WarmClaudeSession {
    pub(crate) async fn is_alive(&self) -> bool {
        self.io.try_lock().map(|mut io| io.child.try_wait().map(|s| s.is_none()).unwrap_or(false)).unwrap_or(true)
    }

    pub(crate) async fn write_json_line(&self, payload: &serde_json::Value) -> Result<(), String> {
        self.activity.touch();
        write_json_stdin(&mut *self.stdin.lock().await, payload).await
    }

    pub(crate) fn kill_now(&self) {
        if self.pid > 0 { kill_process_group(self.pid); }
    }
}

#[derive(Default)]
struct ClaudeSessionPool { sessions: Mutex<HashMap<TopicKey, Arc<WarmClaudeSession>>> }

fn global_pool() -> &'static ClaudeSessionPool {
    static POOL: OnceLock<ClaudeSessionPool> = OnceLock::new();
    POOL.get_or_init(ClaudeSessionPool::default)
}

fn spawn_warm_claude(
    cli: &AntigravityCli,
    ws: PathBuf,
    resume_sid: Option<&str>,
    continue_session: bool,
) -> Result<Arc<WarmClaudeSession>, String> {
    let args = build_warm_command(cli, resume_sid, continue_session);
    let mut cmd = Command::new(&args[0]);
    cmd.args(&args[1..]).current_dir(&ws).envs(cli.build_env())
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).kill_on_drop(true);
    #[cfg(unix)]
    cmd.process_group(0);
    let mut child = cmd.spawn().map_err(|e| format!("Failed to spawn warm claude process: {}", e))?;
    let pid = child.id().unwrap_or(0);
    let stdin = child.stdin.take().ok_or_else(|| "Failed to open claude stdin".to_string())?;
    let stdout = child.stdout.take().ok_or_else(|| "Failed to open claude stdout".to_string())?;
    let (init_m, init_e) = resolve_claude_model_effort(cli.config.model.as_deref(), cli.config.effort.as_deref());
    Ok(Arc::new(WarmClaudeSession {
        pid, session_id: Mutex::new(resume_sid.map(str::to_string)),
        model: Mutex::new(init_m), effort: Mutex::new(init_e), workspace: ws,
        activity: WarmActivityTracker::new(), pending_ask: Mutex::new(None), queued_ask_answers: Mutex::new(None),
        stdin: Mutex::new(stdin), io: Mutex::new(ClaudeProcessIo { child, stdout: BufReader::new(stdout).lines() }),
        running: AtomicBool::new(false), interrupted: AtomicBool::new(false), interrupt_notify: Notify::new(),
    }))
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
    let mut expired = Vec::new();
    for (k, w) in map.iter() {
        if !w.running.load(Ordering::SeqCst) && (w.activity.is_expired() || !w.is_alive().await) {
            expired.push(*k);
        }
    }
    for k in expired {
        if let Some(old) = map.remove(&k) { old.kill_now(); }
    }
}

pub(crate) async fn get_or_spawn_warm_session(
    cli: &AntigravityCli,
    workspace: PathBuf,
    resume_sid: Option<&str>,
    continue_session: bool,
) -> Result<Arc<WarmClaudeSession>, String> {
    cleanup_expired().await;
    let ws = resolve_warm_workspace(cli, workspace);
    crate::cli::common::trust_peer_workspace("claude", &ws, None);
    crate::workspace::skills_helpers::sync_claude_workspace_bridges(&ws);
    let (req_m, req_e) = resolve_claude_model_effort(cli.config.model.as_deref(), cli.config.effort.as_deref());
    let (key, mut eff_res) = ((cli.config.chat_id, cli.config.topic_id), resume_sid.map(str::to_string));
    let mut map = global_pool().sessions.lock().await;
    if let Some(ex) = map.get(&key).cloned() {
        let (cur_s, cur_e) = (ex.session_id.lock().await.clone(), ex.effort.lock().await.clone());
        let sid_ok = resume_sid.is_some_and(|r| cur_s.as_deref() == Some(r)) || continue_session;
        if sid_ok && (req_e.is_none() || req_e == cur_e) && ex.workspace == ws && ex.is_alive().await {
            if let Some(ref m) = req_m && *ex.model.lock().await != Some(m.clone()) {
                let ctrl = serde_json::json!({ "type": "control_request", "request_id": format!("model-{}", m), "request": { "subtype": "set_model", "model": m } });
                if ex.write_json_line(&ctrl).await.is_ok() { *ex.model.lock().await = Some(m.clone()); }
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

/// Sends a soft JSONL `interrupt` control request to the active warm `claude` session for a topic.
pub async fn interrupt_session(chat_id: i64, topic_id: Option<i64>) -> bool {
    let Some(w) = global_pool().sessions.lock().await.get(&(chat_id, topic_id)).cloned() else { return false; };
    if !w.running.load(Ordering::SeqCst) { return false; }
    let ctrl = serde_json::json!({ "type": "control_request", "request_id": format!("stop-{}", chat_id), "request": { "subtype": "interrupt" } });
    let _ = w.write_json_line(&ctrl).await;
    w.interrupted.store(true, Ordering::SeqCst);
    w.interrupt_notify.notify_one();
    true
}

/// Dynamically switches the active model of a warm `claude` session via `set_model` control_request.
pub async fn set_model_live(chat_id: i64, topic_id: Option<i64>, session_id: Option<&str>, model: &str) -> bool {
    let (Some(canonical), _) = resolve_claude_model_effort(Some(model), None) else { return false; };
    let warm = {
        let map = global_pool().sessions.lock().await;
        if let Some(w) = map.get(&(chat_id, topic_id)).cloned() {
            Some(w)
        } else if let Some(target_sid) = session_id.filter(|s| !s.is_empty()) {
            let mut found = None;
            for w in map.values() {
                if w.session_id.lock().await.as_deref() == Some(target_sid) { found = Some(w.clone()); break; }
            }
            found
        } else {
            None
        }
    };
    let Some(w) = warm else { return false; };
    let ctrl = serde_json::json!({ "type": "control_request", "request_id": format!("model-{}", canonical), "request": { "subtype": "set_model", "model": canonical } });
    let ok = w.write_json_line(&ctrl).await.is_ok();
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

/// Returns `true` if a turn is currently running on the warm `claude` session for the topic or `session_id`.
pub async fn is_running(chat_id: i64, topic_id: Option<i64>, session_id: Option<&str>) -> bool {
    let map = global_pool().sessions.lock().await;
    if chat_id == 0 && topic_id.is_none() && session_id.is_none() {
        return map.values().any(|w| w.running.load(Ordering::SeqCst));
    }
    if let Some(w) = map.get(&(chat_id, topic_id)) {
        return w.running.load(Ordering::SeqCst);
    }
    if let Some(sid) = session_id.filter(|s| !s.is_empty()) {
        for w in map.values() {
            if w.session_id.lock().await.as_deref() == Some(sid) {
                return w.running.load(Ordering::SeqCst);
            }
        }
    }
    false
}

/// Returns the number of warm `claude` processes currently registered in the pool.
pub async fn active_count() -> usize {
    global_pool().sessions.lock().await.len()
}
