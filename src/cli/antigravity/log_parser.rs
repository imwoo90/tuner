//! # Log Event Parser for Antigravity Session Logs
//!
//! Parses JSON lines and trace outputs written by agent CLI operations. Emits parsed structured
//! event frames for streaming to messaging interfaces.

//! 
//! ## Search Tags
//! #log-parser

use std::path::Path;
use super::log_helpers::{
    get_new_content_string, parse_entries, parse_ask_question_tool, clean_tool_call_args,
    get_friendly_name, format_thinking, format_bullets
};

fn process_planner_response(
    entry: &serde_json::Value,
    thinking_blocks: &mut Vec<String>,
    tool_calls: &mut Vec<String>,
    final_content: &mut Option<String>,
    ask_question: &mut Option<Vec<crate::cli::AskQuestionData>>,
    status: Option<&str>,
) {
    if let Some(thinking) = entry.get("thinking").and_then(|t| t.as_str()) {
        if !thinking.trim().is_empty() {
            thinking_blocks.push(thinking.trim().to_string());
        }
    }
    if let Some(tcalls) = entry.get("tool_calls").and_then(|t| t.as_array()) {
        for tc in tcalls {
            if let Some(ask) = parse_ask_question_tool(tc) {
                *ask_question = Some(ask);
            }
            tool_calls.push(clean_tool_call_args(tc));
        }
    }
    let tool_calls_empty_or_missing = match entry.get("tool_calls") {
        None => true,
        Some(serde_json::Value::Array(arr)) => arr.is_empty(),
        _ => false,
    };
    if status == Some("DONE") && tool_calls_empty_or_missing {
        if let Some(content) = entry.get("content").and_then(|c| c.as_str()) {
            let trimmed = content.trim();
            if !trimmed.is_empty() && !super::events::is_placeholder_content(trimmed) {
                *final_content = Some(trimmed.to_string());
            }
        }
    }
}

fn process_entry(
    entry: &serde_json::Value,
    thinking_blocks: &mut Vec<String>,
    tool_calls: &mut Vec<String>,
    tool_completions: &mut Vec<String>,
    final_content: &mut Option<String>,
    ask_question: &mut Option<Vec<crate::cli::AskQuestionData>>,
    system_event: &mut bool,
) {
    let source = entry.get("source").and_then(|s| s.as_str());
    let etype = entry.get("type").and_then(|s| s.as_str());
    let status = entry.get("status").and_then(|s| s.as_str());

    if etype == Some("USER_INPUT") {
        thinking_blocks.clear();
        tool_calls.clear();
        tool_completions.clear();
        *final_content = None;
        *ask_question = None;
        return;
    }

    if etype == Some("SYSTEM_MESSAGE") {
        *system_event = true;
    }

    if source == Some("MODEL") {
        if etype == Some("PLANNER_RESPONSE") {
            process_planner_response(entry, thinking_blocks, tool_calls, final_content, ask_question, status);
        } else if status == Some("DONE") {
            if let Some(t) = etype {
                tool_completions.push(format!("`{}` completed", get_friendly_name(t)));
            }
        }
    }
}


fn build_formatted_progress(
    thinking: &[String],
    calls: &[String],
    completions: &[String],
    final_content: Option<&str>,
) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(t) = format_thinking(thinking) {
        parts.push(t);
    }
    if let Some(c) = format_bullets("🛠️ **Tool Calls:", calls) {
        parts.push(c);
    }
    if let Some(comp) = format_bullets("📥 **Tool Completions:", completions) {
        parts.push(comp);
    }
    if let Some(final_ans) = final_content {
        parts.push(final_ans.to_string());
    }

    if !parts.is_empty() {
        Some(parts.join("\n\n"))
    } else {
        None
    }
}


#[derive(Debug, Clone, Default)]
pub struct ParsedLogDelta {
    pub new_size: u64,
    pub formatted: Option<String>,
    pub final_content: Option<String>,
    pub ask_question: Option<Vec<crate::cli::AskQuestionData>>,
    pub has_activity: bool,
}

#[derive(Default)]
pub struct AntigravityLogParser {
    seen_final: bool,
    thinking_blocks: Vec<String>,
    tool_calls: Vec<String>,
    tool_completions: Vec<String>,
}

impl AntigravityLogParser {
    pub fn new() -> Self {
        Self::default()
    }

    fn reset(&mut self) {
        self.seen_final = false;
        self.thinking_blocks.clear();
        self.tool_calls.clear();
        self.tool_completions.clear();
    }

    fn ingest_entries(&mut self, entries: &[serde_json::Value], new_size: u64) -> ParsedLogDelta {
        let prev_counts = (self.thinking_blocks.len(), self.tool_calls.len(), self.tool_completions.len());
        let mut final_content = None;
        let mut ask_question = None;
        let mut system_event = false;

        for entry in entries {
            if entry.get("type").and_then(|s| s.as_str()) == Some("USER_INPUT") {
                self.seen_final = false;
            }
            process_entry(
                entry,
                &mut self.thinking_blocks,
                &mut self.tool_calls,
                &mut self.tool_completions,
                &mut final_content,
                &mut ask_question,
                &mut system_event,
            );
        }

        let counts_now = (self.thinking_blocks.len(), self.tool_calls.len(), self.tool_completions.len());
        let has_activity = system_event || counts_now != prev_counts || final_content.is_some() || ask_question.is_some();

        if self.seen_final {
            final_content = None;
            ask_question = None;
        } else if final_content.is_some() {
            self.seen_final = true;
            ask_question = None;
        }

        let formatted = if has_activity {
            build_formatted_progress(&self.thinking_blocks, &self.tool_calls, &self.tool_completions, final_content.as_deref())
        } else {
            None
        };

        ParsedLogDelta { new_size, formatted, final_content, ask_question, has_activity }
    }

    pub fn parse_log_delta_structured(
        &mut self,
        transcript_path: &Path,
        prev_size: Option<u64>,
    ) -> ParsedLogDelta {
        let (new_content, new_size) = match get_new_content_string(transcript_path, prev_size) {
            Ok(res) => res,
            Err(_) => return ParsedLogDelta { new_size: prev_size.unwrap_or(0), ..Default::default() },
        };
        if prev_size.map(|s| new_size < s).unwrap_or(false) {
            self.reset();
        }
        if new_content.is_empty() {
            return ParsedLogDelta { new_size, ..Default::default() };
        }
        let entries = parse_entries(&new_content);
        if entries.is_empty() {
            return ParsedLogDelta { new_size, ..Default::default() };
        }
        self.ingest_entries(&entries, new_size)
    }

    pub fn parse_log_delta(
        &mut self,
        transcript_path: &Path,
        prev_size: Option<u64>,
    ) -> (u64, Option<String>, Option<Vec<crate::cli::AskQuestionData>>) {
        let res = self.parse_log_delta_structured(transcript_path, prev_size);
        (res.new_size, res.formatted, res.ask_question)
    }
}
