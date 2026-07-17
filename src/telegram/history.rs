use std::path::Path;
use std::fs::{OpenOptions, create_dir_all};
use std::io::Write;
use chrono::Utc;
use serde::Serialize;

#[derive(Serialize)]
pub struct TelegramHistoryEntry {
    pub timestamp: String,
    pub sender: String,
    pub message_id: Option<i32>,
    pub text: String,
    pub is_success: bool,
    pub error: Option<String>,
}

pub fn log_telegram_message(
    working_dir: &Path,
    session_id: &str,
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
    let file_path = target_dir.join("telegram_history.jsonl");
    let entry = TelegramHistoryEntry {
        timestamp: Utc::now().to_rfc3339(),
        sender: sender.to_string(),
        message_id,
        text: text.to_string(),
        is_success,
        error: error.map(|s| s.to_string()),
    };
    if let Ok(serialized) = serde_json::to_string(&entry) {
        if let Ok(mut file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&file_path)
        {
            let _ = writeln!(file, "{}", serialized);
        }
    }
}
