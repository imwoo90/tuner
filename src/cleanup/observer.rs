//! # Storage Purging Observer
//!
//! ## Overview
//! Periodically purges stale files from media and session history folders.
//!
//! ## Collaboration Graph
//! - Ticked by background scheduling timers in the main application loop.
//!
//! ## Search Tags
//! #purger, #disk-maintenance, #retention-policy

use std::fs;
use std::path::{Path, PathBuf};
use chrono::Timelike;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CleanupConfig {
    pub enabled: bool,
    pub media_files_days: u64,
    pub output_to_user_days: u64,
    pub check_hour: u32,
}

impl Default for CleanupConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            media_files_days: 30,
            output_to_user_days: 30,
            check_hour: 3,
        }
    }
}

pub struct CleanupObserver {
    pub config: CleanupConfig,
    pub telegram_files_dir: PathBuf,
    pub output_to_user_dir: PathBuf,
}

/// Recursively delete files older than `max_age_days` inside `path`.
/// Empty subdirectories will also be removed (except the root directory itself).
pub fn delete_old_files(path: &Path, max_age_days: u64) -> u64 {
    if !path.is_dir() {
        return 0;
    }

    let mut deleted_count = 0;
    let now = std::time::SystemTime::now();
    let max_age_duration = std::time::Duration::from_secs(max_age_days * 86400);

    fn walk(
        dir: &Path,
        now: std::time::SystemTime,
        max_age: std::time::Duration,
        deleted_count: &mut u64,
        is_root: bool,
    ) {
        let entries = match fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };

        for entry in entries.flatten() {
            let entry_path = entry.path();
            if entry_path.is_dir() {
                walk(&entry_path, now, max_age, deleted_count, false);
            } else if entry_path.is_file() {
                if let Ok(metadata) = entry.metadata() {
                    if let Ok(modified) = metadata.modified() {
                        if let Ok(age) = now.duration_since(modified) {
                            if age >= max_age {
                                if fs::remove_file(&entry_path).is_ok() {
                                    *deleted_count += 1;
                                }
                            }
                        }
                    }
                }
            }
        }

        // Subdirectory Pruning
        if !is_root {
            if let Ok(mut read_dir) = fs::read_dir(dir) {
                if read_dir.next().is_none() {
                    let _ = fs::remove_dir(dir);
                }
            }
        }
    }

    walk(path, now, max_age_duration, &mut deleted_count, true);
    deleted_count
}

impl CleanupObserver {
    pub fn new(
        config: CleanupConfig,
        telegram_files_dir: PathBuf,
        output_to_user_dir: PathBuf,
    ) -> Self {
        Self {
            config,
            telegram_files_dir,
            output_to_user_dir,
        }
    }

    /// Perform a one-shot file cleanup of configured directories.
    pub async fn execute(&self) -> (u64, u64) {
        let del_tg = delete_old_files(&self.telegram_files_dir, self.config.media_files_days);
        let del_out = delete_old_files(&self.output_to_user_dir, self.config.output_to_user_days);
        (del_tg, del_out)
    }

    /// Start a background monitoring loop checking periodically.
    pub async fn start(self: std::sync::Arc<Self>) {
        if !self.config.enabled {
            return;
        }

        tokio::spawn(async move {
            let mut last_run_date = String::new();
            loop {
                tokio::time::sleep(tokio::time::Duration::from_secs(3600)).await;
                
                let now = chrono::Utc::now();
                let current_hour = now.hour();
                
                if current_hour == self.config.check_hour {
                    let date_str = now.format("%Y-%m-%d").to_string();
                    if last_run_date != date_str {
                        last_run_date = date_str;
                        let (del_tg, del_out) = self.execute().await;
                        println!(
                            "🧹 [tuner] Daily cleanup done. Deleted tg: {}, out: {}",
                            del_tg, del_out
                        );
                    }
                }
            }
        });
    }
}
