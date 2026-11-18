//! # OpenAI Codex Warm & Ephemeral Process Spawner (`spawner.rs`)
//!
//! ## Overview
//! Spawns `codex app-server --stdio` processes in dedicated process groups and executes
//! the JSON-RPC `initialize` + `thread/resume` or `thread/start` handshake.
//!
//! ## Search Tags
//! #codex-spawner, #json-rpc-handshake, #app-server

use super::session::{CodexProcessIo, WarmCodexSession};
use super::{build_app_server_command, resolve_codex_model_effort, DEFAULT_CODEX_MODEL};
use crate::cli::antigravity::AntigravityCli;
use crate::cli::common::{kill_process_group, write_json_stdin, WarmActivityTracker};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, BufReader, Lines};
use tokio::process::{ChildStdin, ChildStdout, Command};
use tokio::sync::{Mutex, Notify};

pub(crate) async fn wait_rpc_response(
    stdout: &mut Lines<BufReader<ChildStdout>>,
    expected_id: u64,
) -> Result<serde_json::Value, String> {
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

pub(crate) fn extract_thread_id(resp: &serde_json::Value) -> Option<String> {
    resp.get("result")?
        .get("thread")?
        .get("id")?
        .as_str()
        .map(str::to_string)
}

async fn send_codex_initialize(
    stdin: &mut ChildStdin,
    stdout: &mut Lines<BufReader<ChildStdout>>,
) -> Result<(), String> {
    write_json_stdin(
        stdin,
        &json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {
                "clientInfo": { "name": "tuner", "version": env!("CARGO_PKG_VERSION") },
                "capabilities": {}
            }
        }),
    )
    .await?;
    let _ = wait_rpc_response(stdout, 1).await?;
    write_json_stdin(
        stdin,
        &json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} }),
    )
    .await
}

async fn handshake_codex_thread(
    cli: &AntigravityCli,
    ws: &Path,
    resume_sid: Option<&str>,
    stdin: &mut ChildStdin,
    stdout: &mut Lines<BufReader<ChildStdout>>,
) -> Result<String, String> {
    send_codex_initialize(stdin, stdout).await?;
    let m = resolve_codex_model_effort(cli.config.model.as_deref(), cli.config.effort.as_deref())
        .0
        .unwrap_or_else(|| DEFAULT_CODEX_MODEL.to_string());
    let (ap, sb) = if cli.config.permission_mode == "bypassPermissions" {
        ("never", "danger-full-access")
    } else {
        ("on-request", "workspace-write")
    };
    let cwd = ws.to_string_lossy();
    if let Some(sid) = resume_sid.filter(|s| !s.is_empty() && !s.starts_with("mock-")) {
        write_json_stdin(
            stdin,
            &json!({
                "jsonrpc": "2.0", "id": 2, "method": "thread/resume",
                "params": { "threadId": sid, "cwd": cwd, "model": m, "approvalPolicy": ap, "sandbox": sb }
            }),
        )
        .await?;
        if let Some(tid) = extract_thread_id(&wait_rpc_response(stdout, 2).await?) {
            return Ok(tid);
        }
        if cli.oneshot {
            return Err(format!("Codex oneshot failed to resume thread '{sid}'"));
        }
    }
    write_json_stdin(
        stdin,
        &json!({
            "jsonrpc": "2.0", "id": 3, "method": "thread/start",
            "params": { "cwd": cwd, "model": m, "approvalPolicy": ap, "sandbox": sb }
        }),
    )
    .await?;
    let resp = wait_rpc_response(stdout, 3).await?;
    extract_thread_id(&resp).ok_or_else(|| format!("Failed to start codex thread: {resp}"))
}

pub(crate) async fn spawn_warm_codex(
    cli: &AntigravityCli,
    ws: PathBuf,
    resume_sid: Option<&str>,
) -> Result<Arc<WarmCodexSession>, String> {
    let args = build_app_server_command(cli);
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
        .map_err(|e| format!("Failed to spawn warm codex app-server: {e}"))?;
    let pid = child.id().unwrap_or(0);
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "Failed to open codex stdin".to_string())?;
    let mut stdout = BufReader::new(
        child
            .stdout
            .take()
            .ok_or_else(|| "Failed to open codex stdout".to_string())?,
    )
    .lines();
    let thread_id = match handshake_codex_thread(cli, &ws, resume_sid, &mut stdin, &mut stdout).await
    {
        Ok(tid) => tid,
        Err(e) => {
            if pid > 0 {
                kill_process_group(pid);
            }
            return Err(e);
        }
    };
    Ok(Arc::new(WarmCodexSession {
        pid,
        thread_id,
        workspace: ws,
        activity: WarmActivityTracker::new(),
        pending_ask: Mutex::new(None),
        queued_ask_answers: Mutex::new(None),
        active_turn_id: Mutex::new(None),
        next_req_id: AtomicU64::new(10),
        stdin: Mutex::new(stdin),
        io: Mutex::new(CodexProcessIo { child, stdout }),
        running: AtomicBool::new(false),
        interrupted: AtomicBool::new(false),
        interrupt_notify: Notify::new(),
    }))
}

pub(crate) async fn prepare_codex_turn_rpc(
    warm: &WarmCodexSession,
    cli: &AntigravityCli,
    prompt: &str,
) -> (u64, serde_json::Value) {
    if prompt.is_empty()
        && let Some((rpc_id, q_ids)) = warm.pending_ask.lock().await.take()
    {
        let ans = warm.queued_ask_answers.lock().await.take().unwrap_or_default();
        return (
            0,
            crate::cli::common::build_codex_ask_rpc_response(rpc_id, &q_ids, &ans),
        );
    }
    let id = warm.next_req_id.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let (m, eff) = resolve_codex_model_effort(cli.config.model.as_deref(), cli.config.effort.as_deref());
    let req = json!({
        "jsonrpc": "2.0", "id": id, "method": "turn/start",
        "params": {
            "threadId": warm.thread_id,
            "input": [{ "type": "text", "text": cli.format_prompt(prompt) }],
            "model": m.unwrap_or_else(|| DEFAULT_CODEX_MODEL.to_string()),
            "effort": eff.unwrap_or_else(|| "medium".to_string()),
        }
    });
    (id, req)
}
