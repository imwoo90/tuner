//! # Log Event Parser for Antigravity Session Logs
//!
//! Parses JSON lines and trace outputs written by agent CLI operations. Emits parsed structured
//! event frames for streaming to messaging interfaces.

use std::path::Path;
use super::log_helpers::{
    get_new_content_string, parse_entries, parse_ask_question_tool, clean_tool_call_args,
    get_friendly_name, format_thinking, format_bullets
};

fn process_entry(
    entry: &serde_json::Value,
    thinking_blocks: &mut Vec<String>,
    tool_calls: &mut Vec<String>,
    tool_completions: &mut Vec<String>,
    final_content: &mut Option<String>,
    ask_question: &mut Option<Vec<crate::cli::AskQuestionData>>,
) {
    let source = entry.get("source").and_then(|s| s.as_str());
    let etype = entry.get("type").and_then(|s| s.as_str());
    let status = entry.get("status").and_then(|s| s.as_str());

    if source == Some("MODEL") {
        if etype == Some("PLANNER_RESPONSE") {
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
        parts.push(format!("✅ **Final Response:**\n{}", final_ans));
    }

    if !parts.is_empty() {
        let header = if final_content.is_some() {
            "**[Ductor Background Completed]**"
        } else {
            "**[Ductor Background Progress]**"
        };
        Some(format!("{}\n\n{}", header, parts.join("\n\n")))
    } else {
        None
    }
}

pub struct AntigravityLogParser {
    seen_final: bool,
}

impl AntigravityLogParser {
    pub fn new() -> Self {
        Self { seen_final: false }
    }

    pub fn parse_log_delta(
        &mut self,
        transcript_path: &Path,
        prev_size: Option<u64>,
    ) -> (u64, Option<String>, Option<Vec<crate::cli::AskQuestionData>>) {
        let (new_content, new_size) = match get_new_content_string(transcript_path, prev_size) {
            Ok(res) => res,
            Err(_) => return (prev_size.unwrap_or(0), None, None),
        };

        let is_truncated = match prev_size {
            Some(size) => new_size < size,
            None => false,
        };
        if is_truncated {
            self.seen_final = false;
        }

        if new_content.is_empty() {
            return (new_size, None, None);
        }

        let entries = parse_entries(&new_content);
        if entries.is_empty() {
            return (new_size, None, None);
        }

        let mut thinking_blocks = Vec::new();
        let mut tool_calls = Vec::new();
        let mut tool_completions = Vec::new();
        let mut final_content = None;
        let mut ask_question = None;

        for entry in &entries {
            process_entry(
                entry,
                &mut thinking_blocks,
                &mut tool_calls,
                &mut tool_completions,
                &mut final_content,
                &mut ask_question,
            );
        }

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

        (new_size, formatted, ask_question)
    }
}
