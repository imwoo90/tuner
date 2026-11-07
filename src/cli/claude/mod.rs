//! # Claude Code CLI Provider (mod.rs)
//!
//! ## Overview
//! Drives Anthropic Claude Code (`claude`) via a persistent warm `--input-format stream-json`
//! and `--output-format stream-json` process per conversation topic, parsing real-time
//! thinking blocks, tool uses, and assistant responses into `StreamEvent` items.
//!
//! ## Search Tags
//! #claude-provider, #stream-json, #warm-process, #agent-executor
//!
//! ## Submodules
//! - [`session`] (`session.rs`): Persistent warm process pool, control-request interrupts, and live model switching.
//! - [`runner`] (`runner.rs`): Asynchronous warm turn executor and stream finalizer.

pub mod runner;
pub mod session;

use crate::cli::antigravity::AntigravityCli;
pub use crate::cli::common::{parse_claude_commands_response, parse_claude_models_response};
use crate::cli::common::{format_stream_preview, probe_cli_json_lines, resolve_cli_bin, strip_effort_suffix};
use crate::cli::{CliResponse, StreamEvent};
pub use runner::{send, send_streaming};
pub use session::{
    active_count, active_pid, cleanup_expired, interrupt_session, is_running, set_model_live,
    submit_ask_answers, terminate_all, terminate_all_for_chat, terminate_topic,
};

/// Fallback Claude Code model identifiers when the CLI binary cannot be probed.
pub const CLAUDE_MODELS: &[&str] = &[
    "claude-opus-4-8",
    "claude-sonnet-5",
    "claude-haiku-4-5",
    "claude-fable-5",
];

/// Default model when switching to the `claude` provider.
pub const DEFAULT_CLAUDE_MODEL: &str = "claude-fable-5";

/// Supported reasoning effort levels for the `claude` CLI.
pub const CLAUDE_EFFORTS: &[&str] = &["low", "medium", "high", "xhigh", "max"];

/// Returns the static fallback model list (with effort suffixes) for the `claude` provider.
pub fn discover_models() -> Vec<String> {
    let mut out: Vec<String> = ["claude-fable-5", "claude-opus-4-8", "claude-sonnet-5"]
        .iter()
        .flat_map(|b| CLAUDE_EFFORTS.iter().map(move |e| format!("{}-{}", b, e)))
        .collect();
    out.push("claude-haiku-4-5".to_string());
    out
}

/// Dynamically queries the `claude` CLI `initialize` control request for the user's live `/model` options.
pub async fn discover_models_live(env: &std::collections::HashMap<String, String>) -> Vec<String> {
    let lines = probe_cli_json_lines(
        &resolve_claude_bin(),
        &["-p", "--input-format", "stream-json", "--output-format", "stream-json", "--verbose"],
        &[r#"{"type":"control_request","request_id":"init_1","request":{"subtype":"initialize"}}"#],
        env,
        5,
    )
    .await;
    lines.iter().find_map(|l| {
        let mut p = parse_claude_models_response(l);
        if p.is_empty() {
            return None;
        }
        if !p.iter().any(|m| m.starts_with("claude-fable-5")) {
            let mut fable: Vec<String> = CLAUDE_EFFORTS.iter().map(|e| format!("claude-fable-5-{}", e)).collect();
            fable.extend(p);
            p = fable;
        }
        Some(p)
    }).unwrap_or_else(discover_models)
}

/// Dynamically queries the `claude` CLI `initialize` control request for live workflow slash commands.
pub async fn discover_commands_live(env: &std::collections::HashMap<String, String>) -> Vec<(String, String)> {
    probe_cli_json_lines(
        &resolve_claude_bin(),
        &["-p", "--input-format", "stream-json", "--output-format", "stream-json", "--verbose"],
        &[r#"{"type":"control_request","request_id":"init_1","request":{"subtype":"initialize"}}"#],
        env,
        5,
    )
    .await
    .iter()
    .find_map(|l| Some(parse_claude_commands_response(l)).filter(|p| !p.is_empty()))
    .unwrap_or_default()
}

/// Resolves the `claude` binary path from `PATH` or user local bin fallback.
pub fn resolve_claude_bin() -> String {
    resolve_cli_bin("claude")
}

fn normalize_claude_model(raw: &str) -> &str {
    match raw {
        "fable" | "claude-fable" => "claude-fable-5",
        "sonnet" | "claude-sonnet" | "claude-sonnet-5-5" | "sonnet-5-5" | "sonnet5.5" => "claude-sonnet-5",
        "opus" | "claude-opus" | "claude-opus-5-5" | "opus-5-5" | "opus5.5" => "claude-opus-4-8",
        "haiku" | "claude-haiku" => "claude-haiku-4-5",
        other => other,
    }
}

/// Resolves the canonical model and effort flags for the `claude` CLI.
pub fn resolve_claude_model_effort(model: Option<&str>, effort: Option<&str>) -> (Option<String>, Option<String>) {
    let mut suffix_eff = None;
    let resolved_model = model.map(str::trim).filter(|s| !s.is_empty() && *s != "antigravity-default").and_then(|raw| {
        let (base, eff) = strip_effort_suffix(raw);
        suffix_eff = eff.map(str::to_string);
        let canonical = normalize_claude_model(base);
        let valid = canonical.starts_with("claude-") || canonical.starts_with("sonnet")
            || canonical.starts_with("opus") || canonical.starts_with("haiku") || canonical.starts_with("fable");
        valid.then(|| canonical.to_string())
    });
    let raw_eff = effort.map(str::trim).filter(|e| !e.is_empty()).map(str::to_lowercase).or(suffix_eff);
    let resolved_eff = raw_eff.and_then(|e| match e.as_str() {
        "minimal" | "low" => Some("low".to_string()),
        "medium" => Some("medium".to_string()),
        "high" => Some("high".to_string()),
        "xhigh" => Some("xhigh".to_string()),
        "max" => Some("max".to_string()),
        _ => None,
    });
    (resolved_model, resolved_eff)
}

/// Builds the persistent warm `claude` command-line vector (`--input-format stream-json`).
pub fn build_warm_command(
    cli: &AntigravityCli,
    resume_session: Option<&str>,
    continue_session: bool,
) -> Vec<String> {
    let mut cmd = vec![
        resolve_claude_bin(),
        "-p".to_string(),
        "--input-format".to_string(),
        "stream-json".to_string(),
        "--output-format".to_string(),
        "stream-json".to_string(),
        "--verbose".to_string(),
        "--permission-prompt-tool".to_string(),
        "stdio".to_string(),
    ];
    if cli.config.permission_mode == "bypassPermissions" {
        cmd.push("--dangerously-skip-permissions".to_string());
    }
    if let Some(sid) = resume_session.filter(|s| !s.is_empty() && !s.starts_with("mock-")) {
        cmd.extend(["--resume".to_string(), sid.to_string()]);
    } else if continue_session {
        cmd.push("--continue".to_string());
    }
    let (m, eff) = resolve_claude_model_effort(cli.config.model.as_deref(), cli.config.effort.as_deref());
    if let Some(model) = m { cmd.extend(["--model".to_string(), model]); }
    if let Some(effort) = eff { cmd.extend(["--effort".to_string(), effort]); }
    if let Some(params) = cli.config.cli_parameters.get("claude") {
        cmd.extend(params.iter().cloned());
    }
    cmd
}

/// Builds the command-line invocation vector for a `claude` CLI turn.
pub fn build_command(
    cli: &AntigravityCli,
    prompt: &str,
    resume_session: Option<&str>,
    continue_session: bool,
) -> Vec<String> {
    let mut cmd = build_warm_command(cli, resume_session, continue_session);
    if !prompt.is_empty() {
        cmd.push(cli.format_prompt(prompt));
    }
    cmd
}

#[derive(Default, Debug, Clone)]
/// Incremental parser for `claude -p --output-format stream-json --verbose` JSONL lines.
pub struct ClaudeStreamParser {
    /// Captured session identifier.
    pub session_id: Option<String>,
    /// Collected thinking blocks in the current turn.
    pub thinking_blocks: Vec<String>,
    /// Collected tool invocation summaries in the current turn.
    pub tool_calls: Vec<String>,
    /// Latest assistant text block.
    pub assistant_text: String,
    /// Final result response if a `"type":"result"` frame was received.
    pub final_result: Option<CliResponse>,
}

impl ClaudeStreamParser {
    /// Creates a new `ClaudeStreamParser`.
    pub fn new() -> Self { Self::default() }

    fn ingest_assistant_blocks(&mut self, blocks: &[serde_json::Value]) -> bool {
        let mut changed = false;
        for b in blocks {
            match b.get("type").and_then(|v| v.as_str()) {
                Some("thinking") => {
                    if let Some(t) = b.get("thinking").or_else(|| b.get("text"))
                        .and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty())
                        && self.thinking_blocks.last().map(String::as_str) != Some(t)
                    {
                        self.thinking_blocks.push(t.to_string());
                        changed = true;
                    }
                }
                Some("tool_use") => {
                    let name = b.get("name").and_then(|v| v.as_str()).unwrap_or("unknown");
                    self.tool_calls.push(format!("🛠 Tool: `{}`", name));
                    changed = true;
                }
                Some("text") => {
                    if let Some(t) = b.get("text").and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty()) {
                        self.assistant_text = t.to_string();
                        changed = true;
                    }
                }
                _ => {}
            }
        }
        changed
    }

    /// Parses a single JSONL line from `claude` stdout and returns any emitted `StreamEvent`s.
    pub fn parse_line(&mut self, line: &str) -> Vec<StreamEvent> {
        let Ok(val) = serde_json::from_str::<serde_json::Value>(line.trim()) else { return Vec::new(); };
        if let Some(sid) = val.get("session_id").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
            self.session_id = Some(sid.to_string());
        }
        let mut events = Vec::new();
        match val.get("type").and_then(|v| v.as_str()) {
            Some("assistant") => {
                if let Some(blocks) = val.get("message").and_then(|m| m.get("content")).and_then(|c| c.as_array())
                    && self.ingest_assistant_blocks(blocks)
                    && let Some(preview) = format_stream_preview(&self.thinking_blocks, &self.tool_calls, &self.assistant_text)
                {
                    events.push(StreamEvent::TextDelta(preview));
                }
            }
            Some("result") => {
                let is_error = val.get("is_error").and_then(|v| v.as_bool()).unwrap_or(false);
                let result = val.get("result").and_then(|v| v.as_str()).map(str::trim)
                    .filter(|s| !s.is_empty()).map(str::to_string)
                    .unwrap_or_else(|| self.assistant_text.clone());
                let resp = CliResponse {
                    session_id: self.session_id.clone(), result, is_error,
                    returncode: Some(if is_error { 1 } else { 0 }), stderr: String::new(),
                };
                self.final_result = Some(resp.clone());
                events.push(StreamEvent::Result(resp));
            }
            _ => {}
        }
        events
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod mod_tests;
