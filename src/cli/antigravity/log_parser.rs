//! # Antigravity CLI Log Delta Parser
//!
//! This module parses transcript JSONL logs incrementally, extracting
//! thinking blocks, tool calls (with sanitized args), tool completions,
//! and final answers for real-time progress updates.

use std::path::Path;

fn read_new_bytes(path: &Path, start_pos: u64) -> Result<(Vec<u8>, u64), String> {
    use std::io::{Read, Seek, SeekFrom};
    let metadata = std::fs::metadata(path).map_err(|e| e.to_string())?;
    let file_size = metadata.len();
    if file_size <= start_pos {
        return Ok((Vec::new(), file_size));
    }
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    file.seek(SeekFrom::Start(start_pos)).map_err(|e| e.to_string())?;
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer).map_err(|e| e.to_string())?;
    Ok((buffer, file_size))
}

fn get_new_content_string(path: &Path, prev_size: Option<u64>) -> Result<(String, u64), String> {
    let metadata = std::fs::metadata(path).map_err(|e| e.to_string())?;
    let file_size = metadata.len();

    let mut start_pos = match prev_size {
        None => return Ok((String::new(), file_size)),
        Some(size) => size,
    };

    if file_size < start_pos {
        start_pos = 0;
    }

    let (bytes, new_size) = read_new_bytes(path, start_pos)?;
    if bytes.is_empty() {
        return Ok((String::new(), new_size));
    }
    Ok((String::from_utf8_lossy(&bytes).to_string(), new_size))
}

fn parse_entries(new_content: &str) -> Vec<serde_json::Value> {
    let mut entries = Vec::new();
    for line in new_content.lines() {
        let stripped = line.trim();
        if !stripped.is_empty() {
            if let Ok(entry) = serde_json::from_str::<serde_json::Value>(stripped) {
                if entry.is_object() {
                    entries.push(entry);
                }
            }
        }
    }
    entries
}

fn format_thinking(thinking_blocks: &[String]) -> Option<String> {
    if thinking_blocks.is_empty() {
        return None;
    }
    let combined = thinking_blocks.join("\n\n");
    let blockquote = combined
        .lines()
        .map(|l| format!(">! {}", l))
        .collect::<Vec<_>>()
        .join("\n");
    Some(format!("💭 **Thinking Process:**\n{}", blockquote))
}

fn format_bullets(title: &str, items: &[String]) -> Option<String> {
    if items.is_empty() {
        return None;
    }
    let list = items
        .iter()
        .map(|item| format!(">! • {}", item))
        .collect::<Vec<_>>()
        .join("\n");
    Some(format!("{}**\n{}", title, list))
}

fn clean_tool_call_args(tc: &serde_json::Value) -> String {
    let name = tc.get("name").and_then(|n| n.as_str()).unwrap_or("unknown");
    let args = tc.get("args").and_then(|a| a.as_object());
    let mut clean_args = serde_json::Map::new();
    if let Some(args_map) = args {
        for (k, v) in args_map {
            if k == "CodeContent"
                || k == "ReplacementContent"
                || k == "ReplacementChunks"
                || k == "TargetContent"
                || (v.as_str().map(|s| s.chars().count() > 200).unwrap_or(false))
            {
                clean_args.insert(k.clone(), serde_json::Value::String("<omitted...>".to_string()));
            } else {
                clean_args.insert(k.clone(), v.clone());
            }
        }
    }
    let args_str = clean_args
        .iter()
        .map(|(k, v)| format!("{}={}", k, serde_json::to_string(v).unwrap_or_default()))
        .collect::<Vec<_>>()
        .join(", ");
    format!("`{}({})`", name, args_str)
}

fn get_friendly_name(t: &str) -> &str {
    match t {
        "RUN_COMMAND" => "run_command (execute)",
        "VIEW_FILE" => "view_file (read)",
        "LIST_DIRECTORY" => "list_dir (list)",
        "GREP_SEARCH" => "grep_search (search)",
        "CODE_ACTION" => "replace_file_content (edit)",
        _ => t,
    }
}

fn process_entry(
    entry: &serde_json::Value,
    thinking_blocks: &mut Vec<String>,
    tool_calls: &mut Vec<String>,
    tool_completions: &mut Vec<String>,
    final_content: &mut Option<String>,
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
                    if !content.trim().is_empty() {
                        *final_content = Some(content.trim().to_string());
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
    ) -> (u64, Option<String>) {
        let (new_content, new_size) = match get_new_content_string(transcript_path, prev_size) {
            Ok(res) => res,
            Err(_) => return (prev_size.unwrap_or(0), None),
        };

        let is_truncated = match prev_size {
            Some(size) => new_size < size,
            None => false,
        };
        if is_truncated {
            self.seen_final = false;
        }

        if new_content.is_empty() {
            return (new_size, None);
        }

        let entries = parse_entries(&new_content);
        if entries.is_empty() {
            return (new_size, None);
        }

        let mut thinking_blocks = Vec::new();
        let mut tool_calls = Vec::new();
        let mut tool_completions = Vec::new();
        let mut final_content = None;

        for entry in &entries {
            process_entry(
                entry,
                &mut thinking_blocks,
                &mut tool_calls,
                &mut tool_completions,
                &mut final_content,
            );
        }

        if self.seen_final {
            final_content = None;
        } else if final_content.is_some() {
            self.seen_final = true;
        }

        let formatted = build_formatted_progress(
            &thinking_blocks,
            &tool_calls,
            &tool_completions,
            final_content.as_deref(),
        );

        (new_size, formatted)
    }
}
