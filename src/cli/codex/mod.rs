//! # OpenAI Codex CLI Provider (mod.rs)
//!
//! ## Overview
//! Drives OpenAI Codex via a persistent warm `codex app-server --stdio` JSON-RPC process per
//! conversation topic, parsing thread IDs, reasoning items, command executions, and streaming
//! agent deltas into `StreamEvent` items.
//!
//! ## Search Tags
//! #codex-provider, #app-server-stdio, #warm-process, #agent-executor
//!
//! ## Submodules
//! - [`session`] (`session.rs`): Persistent warm `codex app-server --stdio` session pool and JSON-RPC handshake.
//! - [`runner`] (`runner.rs`): Asynchronous JSON-RPC `turn/start` executor and stream finalizer.

pub mod runner;
pub mod session;

use crate::cli::antigravity::AntigravityCli;
pub use crate::cli::common::parse_codex_models_response;
use crate::cli::common::{format_stream_preview, probe_cli_json_lines, resolve_cli_bin, strip_effort_suffix};
use crate::cli::StreamEvent;
pub use runner::{send, send_streaming};
pub use session::{
    active_count, active_pid, cleanup_expired, interrupt_session, is_running, submit_ask_answers,
    terminate_all, terminate_all_for_chat, terminate_topic,
};

/// Fallback OpenAI Codex model identifiers when `codex app-server` cannot be probed.
pub const CODEX_MODELS: &[&str] = &[
    "gpt-6.1-sol",
    "gpt-6-astra",
    "gpt-6-sol",
    "gpt-6-luna",
    "gpt-5.6-sol",
    "gpt-6.1-terra",
    "gpt-5.6-terra",
    "gpt-5.6-luna",
    "gpt-5.5",
];

/// Default model when switching to the `codex` provider.
pub const DEFAULT_CODEX_MODEL: &str = "gpt-6-astra";

/// Supported reasoning effort levels for the `codex` CLI.
pub const CODEX_EFFORTS: &[&str] = &["minimal", "low", "medium", "high", "xhigh", "max", "ultra"];

/// Returns the static fallback model list (with effort suffixes) for the `codex` provider.
pub fn discover_models() -> Vec<String> {
    CODEX_MODELS.iter().flat_map(|b| CODEX_EFFORTS.iter().map(move |e| format!("{}-{}", b, e))).collect()
}

/// Dynamically queries `codex app-server --stdio` `model/list` for the user's live `/model` options.
pub async fn discover_models_live(env: &std::collections::HashMap<String, String>) -> Vec<String> {
    let reqs = [
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"clientInfo":{"name":"tuner","version":"0.2.0"}}}"#,
        r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#,
        r#"{"jsonrpc":"2.0","id":2,"method":"model/list","params":{}}"#,
    ];
    probe_cli_json_lines(&resolve_codex_bin(), &["app-server", "--stdio"], &reqs, env, 10)
        .await.iter().find_map(|l| Some(parse_codex_models_response(l)).filter(|p| !p.is_empty()))
        .unwrap_or_else(discover_models)
}

/// Resolves the `codex` binary path from `PATH` or user local bin fallback.
pub fn resolve_codex_bin() -> String { resolve_cli_bin("codex") }

fn normalize_codex_model(raw: &str) -> &str {
    match raw {
        "astra" => "gpt-6-astra",
        "sol" => "gpt-6.1-sol",
        "terra" => "gpt-6.1-terra",
        "luna" => "gpt-6-luna",
        other => other,
    }
}

/// Resolves the canonical model and reasoning effort flags for the `codex` CLI.
pub fn resolve_codex_model_effort(model: Option<&str>, effort: Option<&str>) -> (Option<String>, Option<String>) {
    let mut suffix_eff = None;
    let resolved_model = model.map(str::trim).filter(|s| !s.is_empty() && *s != "antigravity-default").and_then(|raw| {
        let (base, eff) = strip_effort_suffix(raw);
        suffix_eff = eff.map(str::to_string);
        let c = normalize_codex_model(base);
        let ok = (c.starts_with("gpt-") && !c.starts_with("gpt-oss"))
            || ["o1", "o3", "o4", "codex"].iter().any(|p| c.starts_with(p));
        ok.then(|| c.to_string())
    });
    let raw_eff = effort.map(str::trim).filter(|e| !e.is_empty()).map(str::to_lowercase).or(suffix_eff);
    let resolved_eff = raw_eff.and_then(|e| match e.as_str() {
        "minimal" | "low" | "medium" | "high" => Some(e),
        "xhigh" | "max" | "ultra" => Some("xhigh".to_string()),
        _ => None,
    });
    (resolved_model, resolved_eff)
}

/// Builds the persistent warm `codex app-server --stdio` command-line vector.
pub fn build_app_server_command(cli: &AntigravityCli) -> Vec<String> {
    let mut cmd = [
        resolve_codex_bin().as_str(), "app-server", "--stdio",
        "-c", "features.default_mode_request_user_input=true",
        "-c", "suppress_unstable_features_warning=true",
    ].into_iter().map(str::to_string).collect::<Vec<_>>();
    if let Some(params) = cli.config.cli_parameters.get("codex") {
        let mut iter = params.iter();
        while let Some(a) = iter.next() {
            if matches!(a.as_str(), "-c" | "--config" | "--enable" | "--disable") {
                if let Some(v) = iter.next() { cmd.extend([a.clone(), v.clone()]); }
            } else if a.starts_with("-c=") || a.starts_with("--config=") {
                cmd.push(a.clone());
            }
        }
    }
    cmd
}

/// Builds the command-line invocation vector for a `codex` CLI turn.
pub fn build_command(
    cli: &AntigravityCli,
    prompt: &str,
    resume_session: Option<&str>,
    continue_session: bool,
) -> Vec<String> {
    let mut cmd = vec![resolve_codex_bin(), "exec".to_string()];
    if let Some(sid) = resume_session.filter(|s| !s.is_empty() && !s.starts_with("mock-")) {
        cmd.extend(["resume".to_string(), sid.to_string()]);
    } else if continue_session {
        cmd.extend(["resume".to_string(), "--last".to_string()]);
    }
    cmd.extend(["--json".to_string(), "--skip-git-repo-check".to_string()]);
    if cli.config.permission_mode == "bypassPermissions" {
        cmd.push("--dangerously-bypass-approvals-and-sandbox".to_string());
    }
    let (m, eff) = resolve_codex_model_effort(cli.config.model.as_deref(), cli.config.effort.as_deref());
    if let Some(m) = m { cmd.extend(["-m".to_string(), m]); }
    if let Some(e) = eff { cmd.extend(["-c".to_string(), format!("model_reasoning_effort=\"{}\"", e)]); }
    if let Some(p) = cli.config.cli_parameters.get("codex") { cmd.extend(p.iter().cloned()); }
    cmd.push(cli.format_prompt(prompt));
    cmd
}

fn extract_item_text(item: &serde_json::Value) -> Option<String> {
    if let Some(t) = item.get("text").and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty()) {
        return Some(t.to_string());
    }
    for key in ["content", "summary"] {
        if let Some(arr) = item.get(key).and_then(|v| v.as_array()) {
            let parts: Vec<&str> = arr.iter().filter_map(|c| {
                c.as_str().or_else(|| c.get("text").and_then(|v| v.as_str())).map(str::trim).filter(|s| !s.is_empty())
            }).collect();
            if !parts.is_empty() { return Some(parts.join("\n")); }
        }
    }
    None
}

#[derive(Default, Debug, Clone)]
/// Incremental parser for `codex app-server --stdio` JSON-RPC and `codex exec --json` JSONL lines.
pub struct CodexStreamParser {
    /// Captured thread/session identifier.
    pub session_id: Option<String>,
    /// Collected reasoning blocks in the current turn.
    pub thinking_blocks: Vec<String>,
    /// Collected command executions in the current turn.
    pub tool_calls: Vec<String>,
    /// Latest assistant message text.
    pub assistant_text: String,
    /// Captured error message if `turn.failed` or fatal `error` occurred.
    pub error_message: Option<String>,
    /// Whether `turn/completed` was received on an `app-server` turn.
    pub turn_completed: bool,
    delta_buffer: String,
}

impl CodexStreamParser {
    /// Creates a new `CodexStreamParser`.
    pub fn new() -> Self { Self::default() }

    fn preview(&self) -> Option<String> {
        format_stream_preview(&self.thinking_blocks, &self.tool_calls, &self.assistant_text)
    }

    fn ingest_item(&mut self, item: &serde_json::Value) -> bool {
        match item.get("type").and_then(|v| v.as_str()).unwrap_or("") {
            "reasoning" | "Reasoning" => {
                if let Some(t) = extract_item_text(item) && self.thinking_blocks.last() != Some(&t) {
                    self.thinking_blocks.push(t);
                    return true;
                }
            }
            "command_execution" | "CommandExecution" | "commandExecution" => {
                if let Some(c) = item.get("command").and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty()) {
                    let entry = format!("🛠 `{}`", c);
                    if self.tool_calls.last() != Some(&entry) {
                        self.tool_calls.push(entry);
                        return true;
                    }
                }
            }
            "agent_message" | "AgentMessage" | "agentMessage" => {
                self.delta_buffer.clear();
                if let Some(t) = extract_item_text(item) {
                    self.assistant_text = t;
                    return true;
                }
            }
            _ => {}
        }
        false
    }

    fn parse_rpc_method(&mut self, method: &str, params: &serde_json::Value) -> Vec<StreamEvent> {
        let mut ev = Vec::new();
        match method {
            "item/started" | "item/completed" => {
                if let Some(i) = params.get("item") && self.ingest_item(i) && let Some(p) = self.preview() {
                    ev.push(StreamEvent::TextDelta(p));
                }
            }
            "item/agentMessage/delta" => {
                if let Some(d) = params.get("delta").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
                    self.delta_buffer.push_str(d);
                    self.assistant_text = self.delta_buffer.trim().to_string();
                    if let Some(p) = self.preview() { ev.push(StreamEvent::TextDelta(p)); }
                }
            }
            "turn/completed" => {
                self.turn_completed = true;
                if let Some(e) = params.get("turn").and_then(|t| t.get("error")).and_then(|e| e.get("message")).and_then(|v| v.as_str()) {
                    self.error_message = Some(e.to_string());
                }
            }
            "error" => {
                if let Some(m) = params.get("error").and_then(|e| e.get("message")).and_then(|v| v.as_str()) && !m.starts_with("Reconnecting...") {
                    self.error_message = Some(m.to_string());
                }
            }
            _ => {}
        }
        ev
    }

    /// Parses a single JSON line from `codex` stdout and returns any emitted `StreamEvent`s.
    pub fn parse_line(&mut self, line: &str) -> Vec<StreamEvent> {
        let Ok(val) = serde_json::from_str::<serde_json::Value>(line.trim()) else { return Vec::new(); };
        if let Some(tid) = val.get("thread_id").or_else(|| val.get("params").and_then(|p| p.get("threadId"))).and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
            self.session_id = Some(tid.to_string());
        }
        if let Some(m) = val.get("method").and_then(|v| v.as_str()) {
            return self.parse_rpc_method(m, val.get("params").unwrap_or(&serde_json::Value::Null));
        }
        let mut ev = Vec::new();
        match val.get("type").and_then(|v| v.as_str()) {
            Some("item.started" | "item.completed") => {
                if let Some(i) = val.get("item") && self.ingest_item(i) && let Some(p) = self.preview() {
                    ev.push(StreamEvent::TextDelta(p));
                }
            }
            Some("turn.failed") => {
                if let Some(m) = val.get("error").and_then(|e| e.get("message")).and_then(|v| v.as_str()) {
                    self.error_message = Some(m.to_string());
                }
            }
            Some("error") => {
                if let Some(m) = val.get("message").and_then(|v| v.as_str()) && !m.starts_with("Reconnecting...") {
                    self.error_message = Some(m.to_string());
                }
            }
            _ => {}
        }
        ev
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod mod_tests;
