//! # OpenAI Codex Stream Parser (`parser.rs`)
//!
//! ## Overview
//! Incremental parser for `codex app-server --stdio` JSON-RPC notifications and
//! `codex exec --json` JSONL lines, assembling reasoning blocks, command executions,
//! and assistant deltas into `StreamEvent` items.
//!
//! ## Search Tags
//! #codex-parser, #json-rpc-stream, #reasoning-summary

use crate::cli::common::format_stream_preview;
use crate::cli::StreamEvent;

fn extract_item_text(item: &serde_json::Value) -> Option<String> {
    if let Some(t) = item
        .get("text")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return Some(t.to_string());
    }
    for key in ["content", "summary"] {
        if let Some(arr) = item.get(key).and_then(|v| v.as_array()) {
            let parts: Vec<&str> = arr
                .iter()
                .filter_map(|c| {
                    c.as_str()
                        .or_else(|| c.get("text").and_then(|v| v.as_str()))
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                })
                .collect();
            if !parts.is_empty() {
                return Some(parts.join("\n"));
            }
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
    pub(crate) delta_buffer: String,
}

impl CodexStreamParser {
    /// Creates a new `CodexStreamParser`.
    pub fn new() -> Self {
        Self::default()
    }

    fn preview(&self) -> Option<String> {
        format_stream_preview(&self.thinking_blocks, &self.tool_calls, &self.assistant_text)
    }

    fn ingest_item(&mut self, item: &serde_json::Value) -> bool {
        match item.get("type").and_then(|v| v.as_str()).unwrap_or("") {
            "reasoning" | "Reasoning" => {
                if let Some(t) = extract_item_text(item)
                    && self.thinking_blocks.last() != Some(&t)
                {
                    self.thinking_blocks.push(t);
                    return true;
                }
            }
            "command_execution" | "CommandExecution" | "commandExecution" => {
                if let Some(c) = item
                    .get("command")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
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
                if let Some(i) = params.get("item")
                    && self.ingest_item(i)
                    && let Some(p) = self.preview()
                {
                    ev.push(StreamEvent::TextDelta(p));
                }
            }
            "item/agentMessage/delta" => {
                if let Some(d) = params
                    .get("delta")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                {
                    self.delta_buffer.push_str(d);
                    self.assistant_text = self.delta_buffer.trim().to_string();
                    if let Some(p) = self.preview() {
                        ev.push(StreamEvent::TextDelta(p));
                    }
                }
            }
            "turn/completed" => {
                self.turn_completed = true;
                if let Some(e) = params
                    .get("turn")
                    .and_then(|t| t.get("error"))
                    .and_then(|e| e.get("message"))
                    .and_then(|v| v.as_str())
                {
                    self.error_message = Some(e.to_string());
                }
            }
            "error" => {
                if let Some(m) = params
                    .get("error")
                    .and_then(|e| e.get("message"))
                    .and_then(|v| v.as_str())
                    && !m.starts_with("Reconnecting...")
                {
                    self.error_message = Some(m.to_string());
                }
            }
            _ => {}
        }
        ev
    }

    /// Parses a single JSON line from `codex` stdout and returns any emitted `StreamEvent`s.
    pub fn parse_line(&mut self, line: &str) -> Vec<StreamEvent> {
        let Ok(val) = serde_json::from_str::<serde_json::Value>(line.trim()) else {
            return Vec::new();
        };
        if let Some(tid) = val
            .get("thread_id")
            .or_else(|| val.get("params").and_then(|p| p.get("threadId")))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
        {
            self.session_id = Some(tid.to_string());
        }
        if let Some(m) = val.get("method").and_then(|v| v.as_str()) {
            return self.parse_rpc_method(m, val.get("params").unwrap_or(&serde_json::Value::Null));
        }
        let mut ev = Vec::new();
        match val.get("type").and_then(|v| v.as_str()) {
            Some("item.started" | "item.completed") => {
                if let Some(i) = val.get("item")
                    && self.ingest_item(i)
                    && let Some(p) = self.preview()
                {
                    ev.push(StreamEvent::TextDelta(p));
                }
            }
            Some("turn.failed") => {
                if let Some(m) = val
                    .get("error")
                    .and_then(|e| e.get("message"))
                    .and_then(|v| v.as_str())
                {
                    self.error_message = Some(m.to_string());
                }
            }
            Some("error") => {
                if let Some(m) = val.get("message").and_then(|v| v.as_str())
                    && !m.starts_with("Reconnecting...")
                {
                    self.error_message = Some(m.to_string());
                }
            }
            _ => {}
        }
        ev
    }
}
