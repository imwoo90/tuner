//! # Claude Code Warm & Ephemeral Process Spawner (`spawner.rs`)
//!
//! ## Overview
//! Spawns `claude --input-format stream-json --output-format stream-json` OS processes in
//! dedicated process groups for both persistent warm topic sessions and isolated ephemeral runs.
//!
//! ## Search Tags
//! #claude-spawner, #process-group, #stdio-json

use super::{build_warm_command, resolve_claude_model_effort};
use crate::cli::antigravity::AntigravityCli;
use crate::cli::common::{kill_process_group, write_json_stdin, WarmActivityTracker};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::{Mutex, Notify};

pub(crate) struct ClaudeProcessIo {
    pub(crate) child: Child,
    pub(crate) stdout: Lines<BufReader<ChildStdout>>,
}

impl ClaudeProcessIo {
    pub(crate) async fn drain_after_interrupt(&mut self) {
        let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_millis(2500);
        while let Ok(Ok(Some(line))) = tokio::time::timeout_at(deadline, self.stdout.next_line()).await {
            if line.contains("\"type\":\"result\"") {
                break;
            }
        }
    }
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

pub(crate) fn model_ctrl(m: &str) -> serde_json::Value {
    serde_json::json!({
        "type": "control_request",
        "request_id": format!("model-{m}"),
        "request": { "subtype": "set_model", "model": m }
    })
}

pub(crate) fn spawn_warm_claude(
    cli: &AntigravityCli,
    ws: PathBuf,
    resume: Option<&str>,
    cont: bool,
) -> Result<Arc<WarmClaudeSession>, String> {
    let args = build_warm_command(cli, resume, cont);
    let mut cmd = Command::new(&args[0]);
    cmd.args(&args[1..])
        .current_dir(&ws)
        .envs(crate::messenger::web::provider_status::build_peer_env(cli))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(unix)]
    cmd.process_group(0);
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Failed to spawn warm claude process: {e}"))?;
    let pid = child.id().unwrap_or(0);
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| "Failed to open claude stdin".to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Failed to open claude stdout".to_string())?;
    let (init_m, init_e) =
        resolve_claude_model_effort(cli.config.model.as_deref(), cli.config.effort.as_deref());
    Ok(Arc::new(WarmClaudeSession {
        pid,
        session_id: Mutex::new(resume.map(str::to_string)),
        model: Mutex::new(init_m),
        effort: Mutex::new(init_e),
        workspace: ws,
        activity: WarmActivityTracker::new(),
        pending_ask: Mutex::new(None),
        queued_ask_answers: Mutex::new(None),
        stdin: Mutex::new(stdin),
        io: Mutex::new(ClaudeProcessIo {
            child,
            stdout: BufReader::new(stdout).lines(),
        }),
        running: AtomicBool::new(false),
        interrupted: AtomicBool::new(false),
        interrupt_notify: Notify::new(),
    }))
}

pub(crate) async fn prepare_claude_turn_payload(
    warm: &WarmClaudeSession,
    cli: &AntigravityCli,
    prompt: &str,
) -> serde_json::Value {
    if prompt.is_empty()
        && let Some((req_id, input)) = warm.pending_ask.lock().await.take()
    {
        let ans = warm.queued_ask_answers.lock().await.take().unwrap_or_default();
        return crate::cli::common::build_claude_ask_control_response(&req_id, input, &ans);
    }
    serde_json::json!({
        "type": "user",
        "message": { "role": "user", "content": cli.format_prompt(prompt) }
    })
}
