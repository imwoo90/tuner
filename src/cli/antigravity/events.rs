//! # Antigravity CLI Events & Log Parsing
//!
//! This module contains utilities for processing the JSON responses and real-time transcript logs of the Google Antigravity CLI.
//! It supports one-shot response extraction ([`parse_antigravity_json`]) and transcript extraction ([`read_transcript_answer`]).

pub fn parse_antigravity_json(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    match serde_json::from_str::<serde_json::Value>(trimmed) {
        Ok(parsed) => match parsed {
            serde_json::Value::Object(obj) => {
                for key in &["content", "result", "text", "message"] {
                    if let Some(val) = obj.get(*key).and_then(|v| v.as_str()) {
                        if !val.is_empty() {
                            return val.to_string();
                        }
                    }
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

pub fn agy_state_root(env: Option<&HashMap<String, String>>) -> PathBuf {
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
            .unwrap_or_else(|_| PathBuf::from("/home/wimvm"))
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
        if let Some(conv) = obj.get(&key).and_then(|v| v.as_str()) {
            if !conv.is_empty() {
                return Some(conv.to_string());
            }
        }
    }
    None
}

fn newest_brain_dir(brain_root: &Path) -> Option<PathBuf> {
    let entries = std::fs::read_dir(brain_root).ok()?;
    let mut best_dir: Option<PathBuf> = None;
    let mut best_time = std::time::SystemTime::UNIX_EPOCH;

    for entry in entries.flatten() {
        if let Ok(file_type) = entry.file_type() {
            if file_type.is_dir() {
                let path = entry.path();
                let transcript = path
                    .join(".system_generated")
                    .join("logs")
                    .join("transcript_full.jsonl");
                if let Ok(metadata) = transcript.metadata() {
                    if let Ok(modified) = metadata.modified() {
                        if modified > best_time {
                            best_time = modified;
                            best_dir = Some(path);
                        }
                    }
                }
            }
        }
    }
    best_dir
}

pub fn resolve_brain_dir(
    working_dir: &Path,
    env: Option<&HashMap<String, String>>,
) -> Option<PathBuf> {
    let root = agy_state_root(env);
    let brain_root = root.join("brain");

    let is_tuner = env.map(|e| e.contains_key("TUNER_CHAT_ID")).unwrap_or(false);

    if !is_tuner {
        if let Some(conv_id) = conv_id_for_cwd(&root, working_dir) {
            let candidate = brain_root.join(conv_id);
            if candidate.is_dir() {
                return Some(candidate);
            }
        }
    }
    newest_brain_dir(&brain_root)
}

pub fn read_transcript_answer(
    working_dir: &Path,
    env: Option<&HashMap<String, String>>,
    brain_dir: Option<&Path>,
) -> Option<String> {
    let resolved_dir = match brain_dir {
        Some(bd) => bd.to_path_buf(),
        None => resolve_brain_dir(working_dir, env)?,
    };
    let transcript_path = resolved_dir
        .join(".system_generated")
        .join("logs")
        .join("transcript_full.jsonl");
    let bytes = std::fs::read(transcript_path).ok()?;
    let raw = String::from_utf8_lossy(&bytes);
    let mut answer = None;
    for line in raw.lines() {
        let stripped = line.trim();
        if stripped.is_empty() {
            continue;
        }
        if let Ok(entry) = serde_json::from_str::<serde_json::Value>(stripped) {
            if let Some(obj) = entry.as_object() {
                if obj.get("source").and_then(|v| v.as_str()) == Some("MODEL")
                    && obj.get("type").and_then(|v| v.as_str()) == Some("PLANNER_RESPONSE")
                    && obj.get("status").and_then(|v| v.as_str()) == Some("DONE")
                {
                    if let Some(content) = obj.get("content").and_then(|v| v.as_str()) {
                        let content_trimmed = content.trim();
                        if !content_trimmed.is_empty() {
                            answer = Some(content_trimmed.to_string());
                        }
                    }
                }
            }
        }
    }
    answer
}

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





