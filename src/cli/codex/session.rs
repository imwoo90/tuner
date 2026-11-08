//! # OpenAI Codex Persistent Warm Session Pool (session.rs)
//!
//! ## Overview
//! Manages persistent warm `codex app-server --stdio` processes per `(chat_id, topic_id)`,
//! performing JSON-RPC `initialize` + `thread/start` handshakes and `turn/interrupt` signaling.
//!
//! ## Search Tags
//! #codex-session, #app-server-stdio, #warm-process-pool, #json-rpc

use super::{build_app_server_command, resolve_codex_model_effort, DEFAULT_CODEX_MODEL};
use crate::cli::antigravity::AntigravityCli;
use crate::cli::common::{kill_process_group, resolve_warm_workspace, write_json_stdin, WarmActivityTracker};
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use tokio::io::{AsyncBufReadExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::{Mutex, Notify};

type TopicKey = (i64, Option<i64>);

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

async fn wait_rpc_response(stdout: &mut Lines<BufReader<ChildStdout>>, expected_id: u64) -> Result<serde_json::Value, String> {
    let dl = tokio::time::Instant::now() + std::time::Duration::from_secs(15);
    while let Ok(Ok(Some(line))) = tokio::time::timeout_at(dl, stdout.next_line()).await {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(line.trim())
            && val.get("id").and_then(|v| v.as_u64()) == Some(expected_id)
        {
            return Ok(val);
        }
    }
    Err("Codex app-server closed stdout or timed out during RPC handshake".to_string())
}

fn extract_thread_id(resp: &serde_json::Value) -> Option<String> {
    resp.get("result")?.get("thread")?.get("id")?.as_str().map(str::to_string)
}

async fn handshake_codex_thread(
    cli: &AntigravityCli,
    ws: &std::path::Path,
    resume_sid: Option<&str>,
    stdin: &mut ChildStdin,
    stdout: &mut Lines<BufReader<ChildStdout>>,
) -> Result<String, String> {
    write_json_stdin(stdin, &json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": { "clientInfo": { "name": "tuner", "version": env!("CARGO_PKG_VERSION") }, "capabilities": {} }
    })).await?;
    let _ = wait_rpc_response(stdout, 1).await?;
    write_json_stdin(stdin, &json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} })).await?;
    let m = resolve_codex_model_effort(cli.config.model.as_deref(), cli.config.effort.as_deref()).0
        .unwrap_or_else(|| DEFAULT_CODEX_MODEL.to_string());
    let (ap, sb) = if cli.config.permission_mode == "bypassPermissions" { ("never", "danger-full-access") } else { ("on-request", "workspace-write") };
    let cwd = ws.to_string_lossy();
    if let Some(sid) = resume_sid.filter(|s| !s.is_empty() && !s.starts_with("mock-")) {
        write_json_stdin(stdin, &json!({
            "jsonrpc": "2.0", "id": 2, "method": "thread/resume",
            "params": { "threadId": sid, "cwd": cwd, "model": m, "approvalPolicy": ap, "sandbox": sb }
        })).await?;
        if let Some(tid) = extract_thread_id(&wait_rpc_response(stdout, 2).await?) { return Ok(tid); }
    }
    write_json_stdin(stdin, &json!({
        "jsonrpc": "2.0", "id": 3, "method": "thread/start",
        "params": { "cwd": cwd, "model": m, "approvalPolicy": ap, "sandbox": sb }
    })).await?;
    let resp = wait_rpc_response(stdout, 3).await?;
    extract_thread_id(&resp).ok_or_else(|| format!("Failed to start codex thread: {}", resp))
}

async fn spawn_warm_codex(cli: &AntigravityCli, ws: PathBuf, resume_sid: Option<&str>) -> Result<Arc<WarmCodexSession>, String> {
    let args = build_app_server_command(cli);
    let mut cmd = Command::new(&args[0]);
    cmd.args(&args[1..]).current_dir(&ws).envs(cli.build_env())
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).kill_on_drop(true);
    #[cfg(unix)]
    cmd.process_group(0);
    let mut child = cmd.spawn().map_err(|e| format!("Failed to spawn warm codex app-server: {}", e))?;
    let pid = child.id().unwrap_or(0);
    let mut stdin = child.stdin.take().ok_or_else(|| "Failed to open codex stdin".to_string())?;
    let mut stdout = BufReader::new(child.stdout.take().ok_or_else(|| "Failed to open codex stdout".to_string())?).lines();
    let thread_id = match handshake_codex_thread(cli, &ws, resume_sid, &mut stdin, &mut stdout).await {
        Ok(tid) => tid,
        Err(e) => {
            if pid > 0 { kill_process_group(pid); }
            return Err(e);
        }
    };
    Ok(Arc::new(WarmCodexSession {
        pid, thread_id, workspace: ws, activity: WarmActivityTracker::new(),
        pending_ask: Mutex::new(None), queued_ask_answers: Mutex::new(None),
        active_turn_id: Mutex::new(None), next_req_id: AtomicU64::new(10),
        stdin: Mutex::new(stdin), io: Mutex::new(CodexProcessIo { child, stdout }),
        running: AtomicBool::new(false), interrupted: AtomicBool::new(false), interrupt_notify: Notify::new(),
    }))
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
    for k in map.iter().filter(|(_, w)| !w.running.load(Ordering::SeqCst) && w.activity.is_expired()).map(|(k, _)| *k).collect::<Vec<_>>() {
        if let Some(old) = map.remove(&k) { old.kill_now(); }
    }
}

pub(crate) async fn get_or_spawn_warm_codex(
    cli: &AntigravityCli,
    workspace: PathBuf,
    resume_sid: Option<&str>,
    continue_session: bool,
) -> Result<Arc<WarmCodexSession>, String> {
    cleanup_expired().await;
    let (ws, key) = (resolve_warm_workspace(cli, workspace), (cli.config.chat_id, cli.config.topic_id));
    if let Some(ex) = global_pool().sessions.lock().await.get(&key).cloned() {
        if (resume_sid.is_some_and(|r| ex.thread_id == r) || continue_session) && ex.workspace == ws && ex.is_alive().await {
            ex.activity.touch();
            return Ok(ex);
        }
        if let Some(old) = global_pool().sessions.lock().await.remove(&key) { old.kill_now(); }
    }
    let warm = spawn_warm_codex(cli, ws, resume_sid).await?;
    if let Some(old) = global_pool().sessions.lock().await.insert(key, warm.clone()) { old.kill_now(); }
    Ok(warm)
}

/// Sends a JSON-RPC `turn/interrupt` request to the active warm `codex` session for a topic.
pub async fn interrupt_session(chat_id: i64, topic_id: Option<i64>) -> bool {
    let Some(w) = global_pool().sessions.lock().await.get(&(chat_id, topic_id)).cloned() else { return false; };
    if !w.running.load(Ordering::SeqCst) { return false; }
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

/// Returns `true` if a turn is currently running on the warm `codex` session for the topic or `session_id`.
pub async fn is_running(chat_id: i64, topic_id: Option<i64>, session_id: Option<&str>) -> bool {
    let map = global_pool().sessions.lock().await;
    if chat_id == 0 && topic_id.is_none() && session_id.is_none() {
        return map.values().any(|w| w.running.load(Ordering::SeqCst));
    }
    if let Some(w) = map.get(&(chat_id, topic_id)) { return w.running.load(Ordering::SeqCst); }
    session_id.filter(|s| !s.is_empty()).is_some_and(|sid| {
        map.values().any(|w| w.thread_id == sid && w.running.load(Ordering::SeqCst))
    })
}

/// Returns the number of warm `codex` processes currently registered in the pool.
pub async fn active_count() -> usize { global_pool().sessions.lock().await.len() }
