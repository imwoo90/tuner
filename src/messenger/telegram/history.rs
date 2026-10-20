//! # Telegram Chat History logger
//!
//! ## Overview
//! Manages persistent logging of message histories (role, sender, content, thread details)
//! into jsonl files for trace audits, session resume context, and debugging.
//!
//! ## Search Tags
//! #history-logger, #trace-auditing, #jsonl-logs

use std::path::Path;
use std::fs::{OpenOptions, create_dir_all};
use std::io::Write;
use chrono::Utc;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
/// Represents `TelegramHistoryEntry` (telegram history entry).
pub struct TelegramHistoryEntry {
    /// Timestamp.
    pub timestamp: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    /// Topic id.
    pub topic_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    /// Topic name.
    pub topic_name: Option<String>,
    /// Sender.
    pub sender: String,
    /// Message id.
    pub message_id: Option<i32>,
    /// Text.
    pub text: String,
    /// Is success.
    pub is_success: bool,
    /// Error.
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
/// Represents `TopicMetadata` (topic metadata).
pub struct TopicMetadata {
    /// Session id.
    pub session_id: String,
    /// Topic id.
    pub topic_id: Option<i64>,
    /// Topic name.
    pub topic_name: Option<String>,
    /// Created at.
    pub created_at: String,
}

/// Executes `log_telegram_message` (log telegram message).
#[allow(clippy::too_many_arguments)]
pub fn log_telegram_message(
    working_dir: &Path,
    session_id: &str,
    topic_id: Option<i64>,
    topic_name: Option<&str>,
    sender: &str,
    message_id: Option<i32>,
    text: &str,
    is_success: bool,
    error: Option<&str>,
) {
    if session_id.is_empty() {
        return;
    }
    let target_dir = working_dir.join("brain").join(session_id);
    if let Err(e) = create_dir_all(&target_dir) {
        eprintln!("Failed to create directory for telegram history: {:?}", e);
        return;
    }

    // Write topic.json metadata if not already present
    let topic_file = target_dir.join("topic.json");
    if !topic_file.exists() && (topic_id.is_some() || topic_name.is_some()) {
        let meta = TopicMetadata {
            session_id: session_id.to_string(),
            topic_id,
            topic_name: topic_name.map(|s| s.to_string()),
            created_at: Utc::now().to_rfc3339(),
        };
        if let Ok(serialized) = serde_json::to_string_pretty(&meta) {
            let _ = std::fs::write(&topic_file, serialized);
        }
    }

    let file_path = target_dir.join("telegram_history.jsonl");
    let entry = TelegramHistoryEntry {
        timestamp: Utc::now().to_rfc3339(),
        topic_id,
        topic_name: topic_name.map(|s| s.to_string()),
        sender: sender.to_string(),
        message_id,
        text: text.to_string(),
        is_success,
        error: error.map(|s| s.to_string()),
    };
    if let Ok(serialized) = serde_json::to_string(&entry)
        && let Ok(mut file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&file_path)
        {
            let _ = writeln!(file, "{}", serialized);
        }
}

/// Executes `find_sessions_for_topic` (find sessions for topic).
pub fn find_sessions_for_topic(working_dir: &Path, target_topic_id: i64) -> Vec<String> {
    let brain_dir = working_dir.join("brain");
    let mut matching = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&brain_dir) {
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                let topic_file = entry.path().join("topic.json");
                if let Ok(content) = std::fs::read_to_string(&topic_file)
                    && let Ok(meta) = serde_json::from_str::<TopicMetadata>(&content)
                        && meta.topic_id == Some(target_topic_id)
                            && let Some(sid) = entry.file_name().to_str() {
                                matching.push(sid.to_string());
                            }
            }
        }
    }
    matching
}
