//! Task cleanup operations
//!
//! Implements delete, cleanup_old, cleanup_finished, and cleanup_orphans for TaskRegistry.

//! 
//! ## Search Tags
//! #cleanup

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;

use crate::tasks::registry::TaskRegistry;

const FINISHED_STATUSES: &[&str] = &["done", "failed", "cancelled"];

impl TaskRegistry {
    /// Delete a single finished task (entry + folder).
    pub fn delete(&self, task_id: &str) -> anyhow::Result<bool> {
        let mut entries = self.entries.lock().unwrap();
        let entry = match entries.get(task_id) {
            Some(e) => e,
            None => return Ok(false),
        };

        if !FINISHED_STATUSES.contains(&entry.status.as_str()) {
            return Ok(false);
        }

        let folder = self.task_folder_internal(entry);
        entries.remove(task_id);
        self.persist(&entries)?;

        if folder.is_dir() {
            let _ = fs::remove_dir_all(folder);
        }
        Ok(true)
    }

    /// Clean up finished tasks older than max_age_hours.
    pub fn cleanup_old(&self, max_age_hours: i64) -> anyhow::Result<usize> {
        let cutoff = chrono::Utc::now().timestamp() as f64 - (max_age_hours * 3600) as f64;
        let mut entries = self.entries.lock().unwrap();
        let to_remove: Vec<String> = entries
            .iter()
            .filter(|(_, e)| FINISHED_STATUSES.contains(&e.status.as_str()) && e.created_at < cutoff)
            .map(|(k, _)| k.clone())
            .collect();

        let count = to_remove.len();
        if count > 0 {
            let mut folders = HashMap::new();
            for tid in &to_remove {
                if let Some(entry) = entries.get(tid) {
                    folders.insert(tid.clone(), self.task_folder_internal(entry));
                }
            }
            for tid in &to_remove {
                entries.remove(tid);
                if let Some(folder) = folders.get(tid) {
                    if folder.is_dir() {
                        let _ = fs::remove_dir_all(folder);
                    }
                }
            }
            self.persist(&entries)?;
        }
        Ok(count)
    }

    /// Clean up all finished tasks for a specific chat (or all chats if None).
    pub fn cleanup_finished(&self, chat_id: Option<i64>) -> anyhow::Result<usize> {
        let mut entries = self.entries.lock().unwrap();
        let to_remove: Vec<String> = entries
            .iter()
            .filter(|(_, e)| {
                FINISHED_STATUSES.contains(&e.status.as_str())
                    && chat_id.map_or(true, |c| e.chat_id == c)
            })
            .map(|(k, _)| k.clone())
            .collect();

        let count = to_remove.len();
        if count > 0 {
            let mut folders = HashMap::new();
            for tid in &to_remove {
                if let Some(entry) = entries.get(tid) {
                    folders.insert(tid.clone(), self.task_folder_internal(entry));
                }
            }
            for tid in &to_remove {
                entries.remove(tid);
                if let Some(folder) = folders.get(tid) {
                    if folder.is_dir() {
                        let _ = fs::remove_dir_all(folder);
                    }
                }
            }
            self.persist(&entries)?;
        }
        Ok(count)
    }

    /// Cleanup orphan entries and folders.
    pub fn cleanup_orphans(&self) -> anyhow::Result<usize> {
        let mut removed = 0;
        let mut entries = self.entries.lock().unwrap();

        // 1. Registry entry without folder -> drop entry
        let mut to_drop = Vec::new();
        for (tid, entry) in entries.iter() {
            let folder = self.task_folder_internal(entry);
            if !folder.is_dir() {
                to_drop.push(tid.clone());
            }
        }
        for tid in &to_drop {
            entries.remove(tid);
            removed += 1;
        }

        // 2. Folder without registry entry -> delete folder
        let known: HashSet<String> = entries.keys().cloned().collect();
        let mut scan_dirs = HashSet::new();
        scan_dirs.insert(self.default_tasks_dir.clone());
        for entry in entries.values() {
            if !entry.tasks_dir.is_empty() {
                scan_dirs.insert(PathBuf::from(&entry.tasks_dir));
            }
        }

        for tasks_dir in scan_dirs {
            if !tasks_dir.is_dir() {
                continue;
            }
            if let Ok(read_dir) = fs::read_dir(tasks_dir) {
                for entry in read_dir.flatten() {
                    if let Ok(file_type) = entry.file_type() {
                        if file_type.is_dir() {
                            let name = entry.file_name().to_string_lossy().to_string();
                            if !known.contains(&name) {
                                let _ = fs::remove_dir_all(entry.path());
                                removed += 1;
                            }
                        }
                    }
                }
            }
        }

        if removed > 0 {
            self.persist(&entries)?;
        }
        Ok(removed)
    }
}
