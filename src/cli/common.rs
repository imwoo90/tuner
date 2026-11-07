//! # Shared Multi-Provider CLI Utilities (common.rs)
//!
//! ## Overview
//! Provides shared model effort suffix parsing, collapsible stream preview formatting,
//! and process group cleanup utilities used across `antigravity`, `claude`, and `codex`
//! peer CLI drivers.
//!
//! ## Search Tags
//! #cli-common, #stream-preview, #effort-suffix, #multi-provider

pub use crate::cli::antigravity::log_helpers::{format_bullets, format_thinking};
pub use crate::cli::peer::{
    build_claude_ask_control_response, build_codex_ask_rpc_response, is_peer_session_alive,
    parse_claude_ask_questions, parse_claude_commands_response, parse_codex_ask_questions,
    trust_peer_workspace,
};
use crate::cli::antigravity::AntigravityCli;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, Command};

/// Standard 24-hour (`86,400s`) warm session idle expiration window matching Antigravity PTY sessions.
pub const WARM_SESSION_TTL: Duration = Duration::from_secs(86400);

/// Tracks the last active timestamp of a persistent warm CLI process for 24-hour expiration.
pub struct WarmActivityTracker(Mutex<Instant>);

impl Default for WarmActivityTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl WarmActivityTracker {
    /// Creates a new tracker initialized to `Instant::now()`.
    pub fn new() -> Self {
        Self(Mutex::new(Instant::now()))
    }

    /// Updates the last active timestamp to `Instant::now()`.
    pub fn touch(&self) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
    }

    /// Returns `true` if the elapsed idle time since the last activity is at least [`WARM_SESSION_TTL`] (24h).
    pub fn is_expired(&self) -> bool {
        Instant::now().saturating_duration_since(*self.0.lock().unwrap_or_else(|e| e.into_inner())) >= WARM_SESSION_TTL
    }
}

/// Writes a serialized JSON value followed by a newline to a child process `stdin` and flushes it.
pub async fn write_json_stdin(stdin: &mut ChildStdin, payload: &serde_json::Value) -> Result<(), String> {
    let mut line = serde_json::to_string(payload).map_err(|e| e.to_string())?;
    line.push('\n');
    stdin.write_all(line.as_bytes()).await.map_err(|e| format!("CLI stdin write error: {}", e))?;
    stdin.flush().await.map_err(|e| format!("CLI stdin flush error: {}", e))
}

/// Resolves a CLI binary name (`claude`, `codex`) from `PATH` or `~/.local/bin` fallback.
pub fn resolve_cli_bin(name: &str) -> String {
    if let Some(paths) = std::env::var_os("PATH")
        && std::env::split_paths(&paths).any(|d| d.join(name).is_file())
    {
        return name.to_string();
    }
    if let Ok(home) = std::env::var("HOME") {
        let local_bin = PathBuf::from(home).join(".local/bin").join(name);
        if local_bin.is_file() {
            return local_bin.to_string_lossy().to_string();
        }
    }
    let fb = PathBuf::from("/home/wimvm/.local/bin").join(name);
    if fb.is_file() { fb.to_string_lossy().to_string() } else { name.to_string() }
}

/// Resolves the target workspace directory for a warm CLI session.
pub fn resolve_warm_workspace(cli: &AntigravityCli, workspace: PathBuf) -> PathBuf {
    if workspace.as_os_str().is_empty() || workspace == cli.config.working_dir {
        cli.agy_workspace()
    } else {
        workspace
    }
}

/// Sends `SIGKILL` to a Unix process group (`-pid`) and direct process (`pid`).
pub fn kill_process_group(pid: u32) {
    #[cfg(unix)]
    if pid > 0 {
        use nix::sys::signal::{kill, Signal};
        use nix::unistd::Pid;
        let _ = kill(Pid::from_raw(-(pid as i32)), Signal::SIGKILL);
        let _ = kill(Pid::from_raw(pid as i32), Signal::SIGKILL);
    }
}

/// Strips trailing reasoning effort suffixes (`-minimal`, `-low`, `-medium`, `-high`, `-xhigh`, `-max`, `-ultra`)
/// from a combined model identifier string.
pub fn strip_effort_suffix(raw: &str) -> (&str, Option<&'static str>) {
    for (suf, eff) in [
        ("-minimal", "minimal"),
        ("-medium", "medium"),
        ("-xhigh", "xhigh"),
        ("-ultra", "ultra"),
        ("-high", "high"),
        ("-low", "low"),
        ("-max", "max"),
    ] {
        if let Some(base) = raw.strip_suffix(suf) {
            return (base, Some(eff));
        }
    }
    (raw, None)
}

/// Spawns a short-lived CLI process, writes JSON request lines to `stdin`, and collects up to
/// `max_lines` from `stdout` within a 4-second timeout for dynamic model discovery.
pub async fn probe_cli_json_lines(
    bin: &str,
    args: &[&str],
    stdin_lines: &[&str],
    env: &HashMap<String, String>,
    max_lines: usize,
) -> Vec<String> {
    let mut cmd = Command::new(bin);
    cmd.args(args)
        .envs(env)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let Ok(mut child) = cmd.spawn() else { return Vec::new(); };
    if let Some(mut stdin) = child.stdin.take() {
        let payload = format!("{}\n", stdin_lines.join("\n"));
        if stdin.write_all(payload.as_bytes()).await.is_err() || stdin.flush().await.is_err() {
            return Vec::new();
        }
    }
    let Some(stdout) = child.stdout.take() else { return Vec::new(); };
    let mut lines = BufReader::new(stdout).lines();
    let mut out = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(4), async {
        while out.len() < max_lines {
            let Ok(Some(line)) = lines.next_line().await else { break; };
            if !line.trim().is_empty() {
                out.push(line);
            }
        }
    })
    .await;
    let _ = child.kill().await;
    out
}

fn clean_claude_model_id(raw: &str) -> String {
    let s = raw.trim().trim_end_matches("[1m]").trim_end_matches("[2m]");
    if s.len() > 9 {
        let (prefix, tail) = s.split_at(s.len() - 9);
        if tail.starts_with("-202") && tail[1..].chars().all(|c| c.is_ascii_digit()) {
            return prefix.to_string();
        }
    }
    s.to_string()
}

/// Parses the `control_response` line from `claude` `initialize` into selectable model strings.
pub fn parse_claude_models_response(line: &str) -> Vec<String> {
    let Ok(val) = serde_json::from_str::<serde_json::Value>(line.trim()) else { return Vec::new(); };
    let Some(arr) = val
        .get("response")
        .and_then(|r| r.get("response"))
        .and_then(|r| r.get("models"))
        .and_then(|m| m.as_array())
    else {
        return Vec::new();
    };
    let mut seen = Vec::new();
    let mut out = Vec::new();
    for item in arr {
        if item.get("disabled").and_then(|v| v.as_bool()) == Some(true) {
            continue;
        }
        let Some(raw) = item
            .get("resolvedModel")
            .or_else(|| item.get("value"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty() && *s != "default")
        else {
            continue;
        };
        let base = clean_claude_model_id(raw);
        if seen.contains(&base) {
            continue;
        }
        seen.push(base.clone());
        let efforts: Vec<&str> = item
            .get("supportedEffortLevels")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|e| e.as_str()).collect())
            .unwrap_or_default();
        if efforts.is_empty() {
            out.push(base);
        } else {
            out.extend(efforts.into_iter().map(|e| format!("{}-{}", base, e)));
        }
    }
    out
}

/// Parses the `model/list` JSON-RPC response line from `codex app-server --stdio` into selectable model strings.
pub fn parse_codex_models_response(line: &str) -> Vec<String> {
    let Ok(val) = serde_json::from_str::<serde_json::Value>(line.trim()) else { return Vec::new(); };
    let Some(arr) = val.get("result").and_then(|r| r.get("data")).and_then(|d| d.as_array()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for item in arr {
        if item.get("hidden").and_then(|v| v.as_bool()) == Some(true) {
            continue;
        }
        let Some(base) = item
            .get("model")
            .or_else(|| item.get("id"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        let efforts: Vec<&str> = item
            .get("supportedReasoningEfforts")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|e| e.as_str().or_else(|| e.get("reasoningEffort").and_then(|v| v.as_str())))
                    .collect()
            })
            .unwrap_or_default();
        if efforts.is_empty() {
            out.push(base.to_string());
        } else {
            out.extend(efforts.into_iter().map(|e| format!("{}-{}", base, e)));
        }
    }
    out
}

/// Formats incremental thinking blocks, tool call summaries, and assistant text into a unified
/// collapsible preview string for Telegram live streaming.
pub fn format_stream_preview(
    thinking_blocks: &[String],
    tool_calls: &[String],
    assistant_text: &str,
) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(t) = format_thinking(thinking_blocks) {
        parts.push(t);
    }
    if let Some(c) = format_bullets("🛠️ **Tool Calls:", tool_calls) {
        parts.push(c);
    }
    if !assistant_text.is_empty() {
        parts.push(assistant_text.to_string());
    }
    (!parts.is_empty()).then(|| parts.join("\n\n"))
}
