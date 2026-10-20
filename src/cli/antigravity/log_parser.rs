//! # Log Event Parser for Antigravity Session Logs
//!
//! ## Overview
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
    tool_completions: &mut Vec<String>,
    final_content: &mut Option<String>,
    ask_question: &mut Option<Vec<crate::cli::AskQuestionData>>,
    status: Option<&str>,
) {
    let has_thinking = entry.get("thinking").and_then(|t| t.as_str()).is_some_and(|s| !s.trim().is_empty());
    let has_tcalls = entry.get("tool_calls").and_then(|t| t.as_array()).is_some_and(|a| !a.is_empty());
    if has_thinking || has_tcalls {
        thinking_blocks.clear();
        tool_calls.clear();
        if has_tcalls {
            tool_completions.clear();
        }
    }
    if let Some(thinking) = entry.get("thinking").and_then(|t| t.as_str())
        && !thinking.trim().is_empty() {
            thinking_blocks.push(thinking.trim().to_string());
        }
    if let Some(tcalls) = entry.get("tool_calls").and_then(|t| t.as_array()) {
        for tc in tcalls {
            if let Some(ask) = parse_ask_question_tool(tc) {
                *ask_question = Some(ask);
            }
            tool_calls.push(clean_tool_call_args(tc));
        }
    }
    if status == Some("DONE") && !has_tcalls
        && let Some(content) = entry.get("content").and_then(|c| c.as_str()) {
            let trimmed = content.trim();
            if !trimmed.is_empty() && !super::events::is_placeholder_content(trimmed) {
                *final_content = Some(trimmed.to_string());
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
            process_planner_response(entry, thinking_blocks, tool_calls, tool_completions, final_content, ask_question, status);
        } else if status == Some("DONE")
            && let Some(t) = etype {
                tool_completions.push(format!("`{}` completed", get_friendly_name(t)));
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
/// Represents `ParsedLogDelta` (parsed log delta).
pub struct ParsedLogDelta {
    /// New size.
    pub new_size: u64,
    /// Formatted.
    pub formatted: Option<String>,
    /// Final content.
    pub final_content: Option<String>,
    /// Ask question.
    pub ask_question: Option<Vec<crate::cli::AskQuestionData>>,
    /// Has activity.
    pub has_activity: bool,
}

#[derive(Default)]
/// Represents `AntigravityLogParser` (antigravity log parser).
pub struct AntigravityLogParser {
    seen_final: bool,
}

impl AntigravityLogParser {
    /// Creates a new instance.
    pub fn new() -> Self {
        Self::default()
    }

    fn ingest_entries(&mut self, entries: &[serde_json::Value], new_size: u64) -> ParsedLogDelta {
        let mut thinking_blocks = Vec::new();
        let mut tool_calls = Vec::new();
        let mut tool_completions = Vec::new();
        let mut final_content = None;
        let mut ask_question = None;
        let mut system_event = false;

        for entry in entries {
            if entry.get("type").and_then(|s| s.as_str()) == Some("USER_INPUT") {
                self.seen_final = false;
            }
            process_entry(
                entry,
                &mut thinking_blocks,
                &mut tool_calls,
                &mut tool_completions,
                &mut final_content,
                &mut ask_question,
                &mut system_event,
            );
        }

        let has_activity = system_event
            || !thinking_blocks.is_empty()
            || !tool_calls.is_empty()
            || !tool_completions.is_empty()
            || final_content.is_some()
            || ask_question.is_some();

        if self.seen_final {
            final_content = None;
            ask_question = None;
        } else if final_content.is_some() {
            self.seen_final = true;
            ask_question = None;
        }

        let formatted = build_formatted_progress(
            &thinking_blocks,
            &tool_calls,
            &tool_completions,
            final_content.as_deref(),
        );

        ParsedLogDelta { new_size, formatted, final_content, ask_question, has_activity }
    }

    /// Executes `parse_log_delta_structured` (parse log delta structured).
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
            self.seen_final = false;
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

    /// Executes `parse_log_delta` (parse log delta).
    pub fn parse_log_delta(
        &mut self,
        transcript_path: &Path,
        prev_size: Option<u64>,
    ) -> (u64, Option<String>, Option<Vec<crate::cli::AskQuestionData>>) {
        let res = self.parse_log_delta_structured(transcript_path, prev_size);
        (res.new_size, res.formatted, res.ask_question)
    }
}
