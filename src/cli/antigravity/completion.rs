//! # Antigravity CLI Log Completion Checker
//!
//! Provides utilities to inspect transcript full logs for step completion,
//! including tool call completion and interactive tool detection.

use std::path::PathBuf;

pub(crate) fn is_completion_entry(line: &str) -> bool {
    if let Ok(entry) = serde_json::from_str::<serde_json::Value>(line) {
        let source = entry.get("source").and_then(|s| s.as_str());
        let etype = entry.get("type").and_then(|s| s.as_str());
        let status = entry.get("status").and_then(|s| s.as_str());
        let tool_calls_empty = match entry.get("tool_calls") {
            None => true,
            Some(serde_json::Value::Array(arr)) => arr.is_empty(),
            _ => false,
        };
        let has_interactive_tool = match entry.get("tool_calls") {
            Some(serde_json::Value::Array(arr)) => {
                arr.iter().any(|tc| {
                    let name = tc.get("name").and_then(|n| n.as_str());
                    name == Some("ask_question") || name == Some("ask_permission")
                })
            }
            _ => false,
        };
        source == Some("MODEL")
            && etype == Some("PLANNER_RESPONSE")
            && status == Some("DONE")
            && (tool_calls_empty || has_interactive_tool)
    } else {
        false
    }
}

pub(crate) fn check_log_completion_in_file(path: &std::path::Path, current_size: u64) -> Result<Option<u64>, String> {
    if let Ok(metadata) = std::fs::metadata(path) {
        let file_size = metadata.len();
        if file_size > current_size {
            if let Ok(content) = std::fs::read_to_string(path) {
                let mut parser_pos = 0;
                for line in content.lines() {
                    let bytes_len = line.len() + 1;
                    if parser_pos >= current_size && is_completion_entry(line) {
                        return Ok(None);
                    }
                    parser_pos += bytes_len as u64;
                }
            }
            return Ok(Some(file_size));
        }
    }
    Ok(Some(current_size))
}

pub(crate) async fn check_completion_step(
    sessions: &super::session::SessionManager,
    session_id: &str,
    path: Option<&PathBuf>,
    size: &mut u64,
) -> Result<Option<()>, String> {
    let mut holders = sessions.holders.lock().await;
    let is_alive = if let Some(h) = holders.get_mut(session_id) {
        match h.child.try_wait() {
            Ok(None) => true,
            Ok(Some(s)) => {
                if s.success() {
                    if let Some(p) = path {
                        if check_log_completion_in_file(p, *size)?.is_none() {
                            return Ok(Some(()));
                        }
                    }
                    return Ok(Some(()));
                }
                return Err(format!("Process exited with error status: {:?}", s));
            }
            Err(e) => return Err(format!("Failed to check status: {}", e)),
        }
    } else {
        false
    };
    if !is_alive {
        return Err("Process exited prematurely".to_string());
    }
    if let Some(p) = path {
        match check_log_completion_in_file(p, *size)? {
            None => return Ok(Some(())),
            Some(ns) => *size = ns,
        }
    }
    Ok(None)
}
