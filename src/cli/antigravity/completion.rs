//! # Antigravity CLI Log Completion Checker
//!
//! ## Overview
//! Provides utilities to inspect transcript full logs for step completion,
//! including tool call completion and interactive tool detection.
//!
//! ## Search Tags
//! #completion-checker, #transcript-log, #step-status

use std::path::PathBuf;

pub(crate) const THINKING_ONLY_RECOVERY_PROMPT: &str =
    "[SYSTEM RECOVERY] Your previous step ended with thinking only and empty response content. Please immediately output the full response content to the user's last message in the user's language.\r";

pub(crate) fn is_thinking_only_entry(line: &str) -> bool {
    if let Ok(entry) = serde_json::from_str::<serde_json::Value>(line) {
        let source = entry.get("source").and_then(|s| s.as_str());
        let etype = entry.get("type").and_then(|s| s.as_str());
        let status = entry.get("status").and_then(|s| s.as_str());
        let tool_calls_empty = match entry.get("tool_calls") {
            None => true,
            Some(serde_json::Value::Array(arr)) => arr.is_empty(),
            _ => false,
        };
        let has_thinking = entry.get("thinking").and_then(|s| s.as_str()).is_some_and(|s| !s.trim().is_empty());
        let has_content = entry.get("content").and_then(|s| s.as_str()).is_some_and(|s| !s.trim().is_empty());
        source == Some("MODEL")
            && etype == Some("PLANNER_RESPONSE")
            && status == Some("DONE")
            && tool_calls_empty
            && has_thinking
            && !has_content
    } else {
        false
    }
}

pub(crate) fn has_trailing_thinking_only(path: &std::path::Path, start_pos: u64) -> bool {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(meta) = std::fs::metadata(path) else { return false };
    if meta.len() <= start_pos {
        return false;
    }
    let Ok(mut file) = std::fs::File::open(path) else { return false };
    if file.seek(SeekFrom::Start(start_pos)).is_err() {
        return false;
    }
    let mut buf = Vec::new();
    if file.read_to_end(&mut buf).is_err() {
        return false;
    }
    let text = String::from_utf8_lossy(&buf);
    text.lines().rev().find(|l| !l.trim().is_empty()).is_some_and(is_thinking_only_entry)
}

pub(crate) async fn handle_thinking_only_stall(
    sessions: &super::session::SessionManager,
    session_id: &str,
    path: Option<&PathBuf>,
    turn_start_size: &mut u64,
    current_size: u64,
    auto_retried: &mut bool,
) -> Result<bool, String> {
    let Some(p) = path else { return Ok(false) };
    if !has_trailing_thinking_only(p, *turn_start_size) {
        return Ok(false);
    }
    if *auto_retried {
        return Ok(true);
    }
    *auto_retried = true;
    *turn_start_size = current_size;
    if let Some(h) = sessions.holders.lock().await.get_mut(session_id) {
        h.turn_start_offset = h.output.lock().await.len();
    }
    sessions.write_to_session(session_id, THINKING_ONLY_RECOVERY_PROMPT).await?;
    Ok(false)
}

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
        let has_thinking = entry.get("thinking").and_then(|s| s.as_str()).is_some_and(|s| !s.trim().is_empty());
        let has_content = entry.get("content").and_then(|s| s.as_str()).is_some_and(|s| !s.trim().is_empty());
        let is_thinking_only = has_thinking && !has_content && tool_calls_empty;
        source == Some("MODEL")
            && etype == Some("PLANNER_RESPONSE")
            && status == Some("DONE")
            && !is_thinking_only
            && (tool_calls_empty || has_interactive_tool)
    } else {
        false
    }
}

pub(crate) fn check_log_completion_in_file(path: &std::path::Path, current_size: u64) -> Result<Option<u64>, String> {
    if let Ok(metadata) = std::fs::metadata(path) {
        let file_size = metadata.len();
        let start_pos = if file_size < current_size { 0 } else { current_size };
        if file_size > start_pos {
            use std::io::{Read, Seek, SeekFrom};
            if let Ok(mut file) = std::fs::File::open(path)
                && file.seek(SeekFrom::Start(start_pos)).is_ok() {
                    let mut buffer = Vec::new();
                    if file.read_to_end(&mut buffer).is_ok()
                        && let Some(last_nl) = buffer.iter().rposition(|&b| b == b'\n') {
                            let text = String::from_utf8_lossy(&buffer[..=last_nl]);
                            for line in text.lines() {
                                if is_completion_entry(line) {
                                    return Ok(None);
                                }
                            }
                            return Ok(Some(start_pos + last_nl as u64 + 1));
                        }
                }
            return Ok(Some(start_pos));
        }
    }
    Ok(Some(current_size))
}

pub(crate) fn extract_pty_turn_error(raw: &[u8], offset: usize) -> Option<String> {
    use super::pty_spawner::strip_ansi;
    let prior = strip_ansi(&String::from_utf8_lossy(raw.get(..offset)?));
    let clean = strip_ansi(&String::from_utf8_lossy(raw.get(offset..)?));
    let auth_err = "Authentication expired. Please log back in.";
    if clean.contains(auth_err) && !prior.contains(auth_err) {
        return Some(format!(r#"AGY_ERROR: {{"status":"UNAUTHENTICATED","message":"{}"}}"#, auth_err));
    }
    let err_idx = clean.rfind("Error ID:")?;
    let after = &clean[err_idx + "Error ID:".len()..];
    let nl = after.find('\n')?;
    let error_id = after[..nl].trim();
    let has_later_prompt = after[nl + 1..].lines().any(|l| {
        l.trim_start().strip_prefix("> ").is_some_and(|r| !r.trim().is_empty() && !r.trim_start().starts_with('/'))
    });
    if error_id.is_empty() || prior.contains(error_id) || has_later_prompt {
        return None;
    }
    let before = &clean[..err_idx];
    let msg = before.rfind('⚠').map(|w| {
        before[w + '⚠'.len_utf8()..].lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" ")
    }).unwrap_or_else(|| {
        before.lines().map(str::trim).rfind(|l| !l.is_empty() && !l.starts_with('>')).unwrap_or("Antigravity CLI error").to_string()
    });
    let payload = serde_json::json!({ "message": if msg.is_empty() { "Antigravity CLI error" } else { &msg }, "error_id": error_id });
    Some(format!("AGY_ERROR: {}", payload))
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
            Ok(None) => {
                let pty_out = h.output.lock().await;
                if let Some(pty_err) = extract_pty_turn_error(&pty_out, h.turn_start_offset) {
                    return Err(pty_err);
                }
                true
            }
            Ok(Some(s)) => {
                if s.success() {
                    if let Some(p) = path
                        && check_log_completion_in_file(p, *size)?.is_none() {
                            return Ok(Some(()));
                        }
                    return Ok(Some(()));
                }
                let code = exit_status_code(&s).unwrap_or(1);
                return Err(format!("Process exited with error status: exit code {}", code));
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

pub(crate) fn exit_status_code(s: &std::process::ExitStatus) -> Option<i32> {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        s.code().or_else(|| s.signal().map(|sig| 128 + sig))
    }
    #[cfg(not(unix))]
    s.code()
}

