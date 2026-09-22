//! # Review Sessions Persistent Storage
//!
//! Manages long-term (30-day) persistent metadata mapping session IDs to referenced file paths.
//! Synchronizes with Tuner's 30-day cleanup policy, pruning records older than 30 days
//! and validating on-disk file availability.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const RETENTION_SECS: u64 = 30 * 86400; // 30 days

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ReviewSessionRecord {
    pub created_at_secs: u64,
    pub file_paths: Vec<String>,
}

pub fn current_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn default_storage_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".tuner/profiles/default/review_sessions.json")
}

pub fn load_records(path: &Path) -> HashMap<String, ReviewSessionRecord> {
    if !path.is_file() {
        return HashMap::new();
    }
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return HashMap::new(),
    };
    let mut map: HashMap<String, ReviewSessionRecord> = match serde_json::from_str(&content) {
        Ok(m) => m,
        Err(_) => return HashMap::new(),
    };

    let now = current_unix_secs();
    map.retain(|_, rec| now.saturating_sub(rec.created_at_secs) < RETENTION_SECS);
    map
}

pub fn save_records(path: &Path, records: &HashMap<String, ReviewSessionRecord>) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let now = current_unix_secs();
    let mut filtered = records.clone();
    filtered.retain(|_, rec| now.saturating_sub(rec.created_at_secs) < RETENTION_SECS);

    if let Ok(json) = serde_json::to_string_pretty(&filtered) {
        let tmp = path.with_extension("tmp");
        if std::fs::write(&tmp, json).is_ok() {
            let _ = std::fs::rename(&tmp, path);
        }
    }
}

pub fn check_record_files(record: &ReviewSessionRecord) -> (Vec<PathBuf>, usize) {
    let mut valid = Vec::new();
    for p_str in &record.file_paths {
        let p = PathBuf::from(p_str);
        if p.is_file() {
            valid.push(p);
        }
    }
    let total = record.file_paths.len();
    (valid, total)
}
