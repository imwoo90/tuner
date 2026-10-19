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


fn collect_delta_entries(
    entries: &[serde_json::Value],
    seen_final: &mut bool,
) -> (Option<String>, Option<String>, Option<Vec<crate::cli::AskQuestionData>>, bool) {
    let mut thinking_blocks = Vec::new();
    let mut tool_calls = Vec::new();
    let mut tool_completions = Vec::new();
    let mut final_content = None;
    let mut ask_question = None;
    let mut system_event = false;

    for entry in entries {
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

    if *seen_final {
        final_content = None;
        ask_question = None;
    } else if final_content.is_some() {
        *seen_final = true;
        ask_question = None;
    }

    let formatted = build_formatted_progress(
        &thinking_blocks,
        &tool_calls,
        &tool_completions,
        final_content.as_deref(),
    );

    (formatted, final_content, ask_question, has_activity)
}

#[derive(Debug, Clone, Default)]
pub struct ParsedLogDelta {
    pub new_size: u64,
    pub formatted: Option<String>,
    pub final_content: Option<String>,
    pub ask_question: Option<Vec<crate::cli::AskQuestionData>>,
    pub has_activity: bool,
}

pub struct AntigravityLogParser {
    seen_final: bool,
}

impl AntigravityLogParser {
    pub fn new() -> Self {
        Self { seen_final: false }
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
            self.seen_final = false;
        }

        if new_content.is_empty() {
            return ParsedLogDelta { new_size, ..Default::default() };
        }

        let entries = parse_entries(&new_content);
        if entries.is_empty() {
            return ParsedLogDelta { new_size, ..Default::default() };
        }

        let (formatted, final_content, ask_question, has_activity) =
            collect_delta_entries(&entries, &mut self.seen_final);

        ParsedLogDelta {
            new_size,
            formatted,
            final_content,
            ask_question,
            has_activity,
        }
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
