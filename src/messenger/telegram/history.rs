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
pub fn log_telegram_message(
    working_dir: &Path,
    session_id: &str,
    topic: (Option<i64>, Option<&str>),
    sender: &str,
    message_id: Option<i32>,
    text: &str,
    status: (bool, Option<&str>),
) {
    let (topic_id, topic_name) = topic;
    let (is_success, error) = status;
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

pub(crate) fn capture_pre_transcript_cursor(
    cc: &crate::cli::antigravity::AntigravityCli,
    sess: &crate::session::data::SessionData,
    sid: &str,
    default_prov: &str,
) -> Option<(std::path::PathBuf, u64)> {
    let prov = crate::session::data::SessionData::effective_provider(&sess.provider, default_prov);
    if prov != "antigravity" || sid.is_empty() {
        return None;
    }
    let tp = crate::cli::antigravity::events::agy_state_root(Some(&cc.build_env()))
        .join("brain")
        .join(sid)
        .join(".system_generated/logs/transcript_full.jsonl");
    let pre_len = std::fs::metadata(&tp).map(|m| m.len()).unwrap_or(0);
    Some((tp, pre_len))
}

pub(crate) async fn recover_missed_agy_ask(
    bot: &teloxide::Bot,
    msg: &teloxide::types::Message,
    pre: Option<&(std::path::PathBuf, u64)>,
    sessions: &crate::session::manager::SessionManager,
    sess: &crate::session::data::SessionData,
    config: &crate::config::CliConfig,
    cli: &crate::cli::antigravity::AntigravityCli,
) -> Result<(), teloxide::RequestError> {
    let Some((tp, pre_len)) = pre else { return Ok(()) };
    let delta = crate::cli::antigravity::log_parser::AntigravityLogParser::new()
        .parse_log_delta_structured(tp, Some(*pre_len));
    if let Some(ask) = delta.ask_question {
        let latest = sessions.get_active(&sess.session_key()).await.ok().flatten().unwrap_or_else(|| sess.clone());
        super::ask_process::handle_ask_question_event(
            bot, msg.chat.id, msg.thread_id.map(|t| t.0.0), ask, &latest, (config, cli, sessions),
        ).await?;
    }
    Ok(())
}
