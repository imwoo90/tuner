//! # Antigravity CLI Events & Log Parsing
//!
//! ## Overview
//! This module contains utilities for processing the JSON responses and real-time transcript logs of the Google Antigravity CLI.
//! It supports one-shot response extraction ([`parse_antigravity_json`]) and transcript extraction ([`read_transcript_answer`]).
//!
//! ## Search Tags
//! #events

/// Executes `parse_antigravity_json` (parse antigravity json).
pub fn parse_antigravity_json(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    match serde_json::from_str::<serde_json::Value>(trimmed) {
        Ok(parsed) => match parsed {
            serde_json::Value::Object(obj) => {
                let reason = obj.get("finished_reason").and_then(|v| v.as_str()).filter(|s| !s.is_empty());
                for key in &["content", "result", "text", "message", "response"] {
                    if let Some(val) = obj.get(*key).and_then(|v| v.as_str())
                        && !val.is_empty() {
                            return match reason {
                                Some(r) => format!("{val}\n\n⚠️ [Turn ended early: {r}]"),
                                None => val.to_string(),
                            };
                        }
                }
                if let Some(r) = reason {
                    return format!("⚠️ Turn ended early: {r}");
                }
                serde_json::Value::Object(obj).to_string()
            }
            serde_json::Value::String(s) => s,
            other => other.to_string(),
        },
        Err(_) => {
            if trimmed.len() > 2000 {
                trimmed.chars().take(2000).collect()
            } else {
                trimmed.to_string()
            }
        }
    }
}


use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Executes `agy_state_root` (agy state root).
pub fn agy_state_root(env: Option<&HashMap<String, String>>) -> PathBuf {
    if let Some(e) = env
        && let Some(root) = e.get("AGY_STATE_ROOT") {
            return PathBuf::from(root);
        }
    if let Ok(root) = std::env::var("AGY_STATE_ROOT") {
        return PathBuf::from(root);
    }
    let home = if let Some(e) = env {
        e.get("USERPROFILE")
            .cloned()
            .or_else(|| e.get("HOME").cloned())
    } else {
        std::env::var("USERPROFILE")
            .ok()
            .or_else(|| std::env::var("HOME").ok())
    };

    let base = if let Some(h) = home {
        PathBuf::from(h)
    } else {
        std::env::var("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("."))
    };

    base.join(".gemini").join("antigravity-cli")
}

fn conv_id_for_cwd(root: &Path, working_dir: &Path) -> Option<String> {
    let mapping_path = root.join("cache").join("last_conversations.json");
    let content = std::fs::read_to_string(mapping_path).ok()?;
    let mapping: serde_json::Value = serde_json::from_str(&content).ok()?;
    let obj = mapping.as_object()?;

    let keys = vec![
        working_dir.to_string_lossy().to_string(),
        working_dir
            .canonicalize()
            .unwrap_or_else(|_| working_dir.to_path_buf())
            .to_string_lossy()
            .to_string(),
    ];
    for key in keys {
        if let Some(conv) = obj.get(&key).and_then(|v| v.as_str())
            && !conv.is_empty() {
                return Some(conv.to_string());
            }
    }
    None
}

fn newest_brain_dir(brain_root: &Path) -> Option<PathBuf> {
    let entries = std::fs::read_dir(brain_root).ok()?;
    let mut best_dir: Option<PathBuf> = None;
    let mut best_time = std::time::SystemTime::UNIX_EPOCH;

    for entry in entries.flatten() {
        if let Ok(file_type) = entry.file_type()
            && file_type.is_dir() {
                let path = entry.path();
                let transcript = path
                    .join(".system_generated")
                    .join("logs")
                    .join("transcript_full.jsonl");
                if let Ok(metadata) = transcript.metadata()
                    && let Ok(modified) = metadata.modified()
                        && modified > best_time {
                            best_time = modified;
                            best_dir = Some(path);
                        }
            }
    }
    best_dir
}

/// Executes `resolve_brain_dir` (resolve brain dir).
pub fn resolve_brain_dir(
    working_dir: &Path,
    env: Option<&HashMap<String, String>>,
) -> Option<PathBuf> {
    let root = agy_state_root(env);
    let brain_root = root.join("brain");

    if let Some(conv_id) = conv_id_for_cwd(&root, working_dir) {
        let candidate = brain_root.join(conv_id);
        if candidate.is_dir() {
            return Some(candidate);
        }
    }
    newest_brain_dir(&brain_root)
}

fn update_answer_from_entry(
    obj: &serde_json::Map<String, serde_json::Value>,
    answer: &mut Option<String>,
    fallback_thinking: &mut Option<String>,
) {
    if matches!(obj.get("type").and_then(|v| v.as_str()), Some("USER_INPUT" | "SYSTEM_MESSAGE")) {
        if obj.get("content").and_then(|v| v.as_str()).is_none_or(|c| !c.contains("[SYSTEM RECOVERY]")) {
            *answer = None;
            *fallback_thinking = None;
        }
    } else if obj.get("source").and_then(|v| v.as_str()) == Some("MODEL")
        && obj.get("type").and_then(|v| v.as_str()) == Some("PLANNER_RESPONSE")
        && obj.get("status").and_then(|v| v.as_str()) == Some("DONE")
    {
        if let Some(c) = obj.get("content").and_then(|v| v.as_str()).map(str::trim)
            && !c.is_empty() && !is_placeholder_content(c)
        {
            *answer = Some(c.to_string());
        }
        if answer.is_none()
            && let Some(t) = obj.get("thinking").and_then(|v| v.as_str()).map(str::trim)
            && !t.is_empty()
        {
            *fallback_thinking = Some(t.to_string());
        }
    }
}

/// Executes `read_transcript_answer` (read transcript answer).
pub fn read_transcript_answer(
    working_dir: &Path,
    env: Option<&HashMap<String, String>>,
    brain_dir: Option<&Path>,
) -> Option<String> {
    let resolved_dir = match brain_dir {
        Some(bd) => bd.to_path_buf(),
        None => resolve_brain_dir(working_dir, env)?,
    };
    let tp = resolved_dir.join(".system_generated/logs/transcript_full.jsonl");
    let raw = String::from_utf8_lossy(&std::fs::read(tp).ok()?).into_owned();
    let (mut answer, mut fallback_thinking) = (None, None);
    for line in raw.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if let Ok(serde_json::Value::Object(obj)) = serde_json::from_str(line) {
            update_answer_from_entry(&obj, &mut answer, &mut fallback_thinking);
        }
    }
    answer.or(fallback_thinking)
}

/// Executes `is_placeholder_content` (is placeholder content).
pub fn is_placeholder_content(content: &str) -> bool {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return true;
    }
    let without_comments = remove_html_comments_and_nbsp(trimmed);
    without_comments.trim().is_empty()
}

fn remove_html_comments_and_nbsp(s: &str) -> String {
    let mut result = String::new();
    let mut in_comment = false;
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if !in_comment && i + 4 <= chars.len() && chars[i..i+4] == ['<', '!', '-', '-'] {
            in_comment = true;
            i += 4;
        } else if in_comment && i + 3 <= chars.len() && chars[i..i+3] == ['-', '-', '>'] {
            in_comment = false;
            i += 3;
        } else if !in_comment {
            result.push(chars[i]);
            i += 1;
        } else {
            i += 1;
        }
    }
    result.replace("&nbsp;", "").replace("&#160;", "")
}

/// Executes `extract_conversation_id` (extract conversation id).
pub fn extract_conversation_id(stdout: &str) -> Option<String> {
    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('{')
            && let Ok(val) = serde_json::from_str::<serde_json::Value>(trimmed)
                && let Some(cid) = val.get("conversation_id").and_then(|s| s.as_str())
                    && !cid.is_empty() {
                        return Some(cid.to_string());
                    }
    }
    None
}

/// Extracts `finished_reason` (`print_timeout`, `interrupted`, `error`) from `agy` JSON output.
pub fn extract_finished_reason(stdout: &str) -> Option<String> {
    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('{')
            && let Ok(val) = serde_json::from_str::<serde_json::Value>(trimmed)
                && let Some(reason) = val.get("finished_reason").and_then(|s| s.as_str())
                    && !reason.is_empty() {
                        return Some(reason.to_string());
                    }
    }
    None
}

/// Returns the live installed `agy --version` paired with the compile-time verified baseline.
pub fn agy_version_summary() -> Option<String> {
    let out = std::process::Command::new("agy").arg("--version").output().ok()?;
    Some(format!("{}, verified: {}", String::from_utf8_lossy(&out.stdout).trim(), env!("TUNER_VERIFIED_AGY_VERSION")))
}





