//! # Log Helper Utilities for Agent Outputs
//!
//! Provides diagnostic utilities to extract raw text blocks from active session log directories,
//! helping inspect model trace buffers and parser state.

//! 
//! ## Search Tags
//! #log-helpers

use std::path::Path;

pub fn read_new_bytes(path: &Path, start_pos: u64) -> Result<(Vec<u8>, u64), String> {
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

pub fn get_new_content_string(path: &Path, prev_size: Option<u64>) -> Result<(String, u64), String> {
    let metadata = std::fs::metadata(path).map_err(|e| e.to_string())?;
    let file_size = metadata.len();

    let mut start_pos = match prev_size {
        None => 0,
        Some(size) => size,
    };

    if file_size < start_pos {
        start_pos = 0;
    }

    let (bytes, read_to_size) = read_new_bytes(path, start_pos)?;
    if bytes.is_empty() {
        return Ok((String::new(), read_to_size));
    }

    if let Some(last_nl) = bytes.iter().rposition(|&b| b == b'\n') {
        let valid_bytes = &bytes[..=last_nl];
        let new_pos = start_pos + (last_nl as u64) + 1;
        Ok((String::from_utf8_lossy(valid_bytes).to_string(), new_pos))
    } else if bytes.len() > 64 * 1024 {
        Ok((String::from_utf8_lossy(&bytes).to_string(), read_to_size))
    } else {
        Ok((String::new(), start_pos))
    }
}

pub fn parse_entries(new_content: &str) -> Vec<serde_json::Value> {
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

pub fn format_thinking(thinking_blocks: &[String]) -> Option<String> {
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

pub fn format_bullets(title: &str, items: &[String]) -> Option<String> {
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

pub fn clean_tool_call_args(tc: &serde_json::Value) -> String {
    let name = tc.get("name").and_then(|n| n.as_str()).unwrap_or("unknown");
    let args = tc.get("args").and_then(|a| a.as_object());
    let mut clean_args = serde_json::Map::new();
    if let Some(args_map) = args {
        for (k, v) in args_map {
            let omit = ["CodeContent", "ReplacementContent", "ReplacementChunks", "TargetContent"].contains(&k.as_str())
                || v.as_str().map(|s| s.chars().count() > 200).unwrap_or(false);
            if omit {
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

pub fn get_friendly_name(t: &str) -> &str {
    match t {
        "RUN_COMMAND" => "run_command (execute)",
        "VIEW_FILE" => "view_file (read)",
        "LIST_DIRECTORY" => "list_dir (list)",
        "GREP_SEARCH" => "grep_search (search)",
        "CODE_ACTION" => "replace_file_content (edit)",
        _ => t,
    }
}

pub fn parse_ask_question_tool(tc: &serde_json::Value) -> Option<Vec<crate::cli::AskQuestionData>> {
    let name = tc.get("name").and_then(|n| n.as_str()).unwrap_or("");
    if name == "ask_question" {
        if let Some(args) = tc.get("args") {
            let questions_array = args.get("questions").and_then(|q| {
                if q.is_array() {
                    q.as_array().cloned()
                } else {
                    q.as_str()
                        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
                        .and_then(|v| v.as_array().cloned())
                }
            });
            if let Some(questions) = questions_array {
                let mut parsed_questions = Vec::new();
                for q in questions {
                    let question = q.get("question").and_then(|q| q.as_str()).unwrap_or("").to_string();
                    let options: Vec<String> = q.get("options")
                        .and_then(|opts| opts.as_array())
                        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
                        .unwrap_or_default();
                    let is_multi_select = q.get("is_multi_select").and_then(|b| b.as_bool()).unwrap_or(false);
                    if !question.is_empty() && !options.is_empty() {
                        parsed_questions.push(crate::cli::AskQuestionData {
                            question,
                            options,
                            is_multi_select,
                        });
                    }
                }
                if !parsed_questions.is_empty() {
                    return Some(parsed_questions);
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_get_new_content_string_holds_offset_on_incomplete_line() {
        let temp_dir = tempfile::tempdir().unwrap();
        let log_path = temp_dir.path().join("transcript.jsonl");

        let mut file = std::fs::File::create(&log_path).unwrap();
        file.write_all(b"{\"step_index\": 1, \"content\": \"incomplete").unwrap();
        file.flush().unwrap();

        let (content, pos) = get_new_content_string(&log_path, None).unwrap();
        assert_eq!(content, "");
        assert_eq!(pos, 0);

        file.write_all(b"_line\"}\n").unwrap();
        file.flush().unwrap();

        let (content2, pos2) = get_new_content_string(&log_path, Some(pos)).unwrap();
        assert_eq!(content2, "{\"step_index\": 1, \"content\": \"incomplete_line\"}\n");
        assert_eq!(pos2 as usize, b"{\"step_index\": 1, \"content\": \"incomplete_line\"}\n".len());
    }

    #[test]
    fn test_get_new_content_string_advances_only_to_last_newline() {
        let temp_dir = tempfile::tempdir().unwrap();
        let log_path = temp_dir.path().join("transcript.jsonl");

        let mut file = std::fs::File::create(&log_path).unwrap();
        file.write_all(b"{\"line\": 1}\n{\"line\": 2, \"incomplete").unwrap();
        file.flush().unwrap();

        let (content, pos) = get_new_content_string(&log_path, None).unwrap();
        assert_eq!(content, "{\"line\": 1}\n");
        assert_eq!(pos as usize, b"{\"line\": 1}\n".len());

        file.write_all(b"\"}\n").unwrap();
        file.flush().unwrap();

        let (content2, pos2) = get_new_content_string(&log_path, Some(pos)).unwrap();
        assert_eq!(content2, "{\"line\": 2, \"incomplete\"}\n");
        assert_eq!(pos2 as usize, b"{\"line\": 1}\n{\"line\": 2, \"incomplete\"}\n".len());
    }
}
