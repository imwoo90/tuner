//! # Workspace Folder Synchronizer
//!
//! ## Overview
//! Initializes workspace layout folders (cron_tasks, telegram_files, skills) and mounts rules profiles.
//!
//! ## Collaboration Graph
//! - Invoked during application initialization in [`main.rs`](crate::main.rs).
//!
//! ## Search Tags
//! #workspace-init, #profile-copy, #environment-setup

use crate::workspace::paths::DuctorPaths;
use crate::workspace::sync_helpers::{
    create_workspace_directories, migrate_legacy_data, smart_merge_config, sync_constraints_rule,
    sync_group, sync_mainmemory_rule, sync_rule_files_recursive, walk_and_copy,
};
use std::path::Path;


/// Initializes the workspace directory structure and configurations.
pub fn init_workspace(paths: &DuctorPaths) -> Result<(), String> {
    if paths.profile.is_none() {
        let _ = std::fs::create_dir_all(paths.config_dir());
        let _ = smart_merge_config(paths);
        return Ok(());
    }

    migrate_legacy_data(paths);

    let old_tasks = paths.workspace().join("tasks");
    if old_tasks.is_dir() && !paths.cron_tasks_dir().exists() {
        let _ = std::fs::rename(&old_tasks, paths.cron_tasks_dir());
    }

    let _ = crate::workspace::skills::sync_bundled_skills(paths, false);

    if paths.home_defaults.is_dir() {
        walk_and_copy(&paths.home_defaults, &paths.profile_home(), &paths.home_defaults)?;
    }

    create_workspace_directories(paths);
    let _ = sync_mainmemory_rule(&paths.workspace());
    let default_constraints = paths.home_defaults.join("workspace").join(".agents").join("rules").join("constraints.md");
    let _ = sync_constraints_rule(&paths.workspace(), Some(&default_constraints));

    let selector = crate::workspace::rules::RulesSelector::new(paths.clone());
    let _ = selector.deploy_rules();

    let _ = crate::workspace::sync_helpers::smart_merge_profile_config(paths);

    let _ = ensure_task_rule_files(&paths.cron_tasks_dir());
    let _ = sync_rule_files(&paths.workspace());
    if paths.profile.is_none() {
        let _ = smart_merge_config(paths);
    }

    if paths.workspace().is_dir() {
        if let Ok(entries) = std::fs::read_dir(paths.workspace()) {
            for entry in entries {
                if let Ok(entry) = entry {
                    let path = entry.path();
                    if path.is_symlink() && !path.exists() {
                        let _ = std::fs::remove_file(&path);
                    }
                }
            }
        }
    }

    let _ = crate::workspace::skills::sync_skills(paths, false);
    Ok(())
}


/// Recursively synchronizes existing rule files across the workspace.
pub fn sync_rule_files(root: &Path) -> Result<(), String> {
    if !root.is_dir() {
        return Ok(());
    }
    sync_group(root)?;
    sync_rule_files_recursive(root)?;
    let _ = sync_mainmemory_rule(root);
    let _ = sync_constraints_rule(root, None);
    Ok(())
}

/// Asynchronously monitors changes to rule files and syncs them periodically.
pub async fn watch_rule_files(root: &Path, interval_ms: u64) -> Result<(), String> {
    let mut interval = tokio::time::interval(tokio::time::Duration::from_millis(interval_ms));
    let cron_tasks_dir = root.join("cron_tasks");
    loop {
        interval.tick().await;
        let root_clone = root.to_path_buf();
        let cron_tasks_clone = cron_tasks_dir.clone();
        let _ = tokio::task::spawn_blocking(move || {
            let _ = ensure_task_rule_files(&cron_tasks_clone);
            let _ = sync_rule_files(&root_clone);
        }).await;
    }
}

pub fn ensure_task_rule_files(cron_tasks_dir: &Path) -> Result<usize, String> {
    if !cron_tasks_dir.is_dir() {
        return Ok(0);
    }
    let expected = detect_rule_filenames(cron_tasks_dir);
    let mut created = 0;
    let rule_filenames = ["CLAUDE.md", "AGENTS.md", "GEMINI.md"];

    let entries = match std::fs::read_dir(cron_tasks_dir) {
        Ok(e) => e,
        Err(e) => return Err(format!("Failed to read dir: {}", e)),
    };

    for entry in entries {
        if let Ok(entry) = entry {
            let task_dir = entry.path();
            if task_dir.is_dir() {
                let mut existing = Vec::new();
                for name in &rule_filenames {
                    if task_dir.join(name).is_file() {
                        existing.push(*name);
                    }
                }
                if existing.is_empty() {
                    continue;
                }
                let mut missing = Vec::new();
                for name in &expected {
                    if !task_dir.join(name).is_file() {
                        missing.push(name.clone());
                    }
                }
                if missing.is_empty() {
                    continue;
                }
                if let Ok(source_content) = std::fs::read_to_string(task_dir.join(existing[0])) {
                    for name in missing {
                        let _ = std::fs::write(task_dir.join(name), &source_content);
                        created += 1;
                    }
                }
            }
        }
    }
    Ok(created)
}

fn detect_rule_filenames(cron_tasks_dir: &Path) -> Vec<String> {
    let rule_filenames = ["CLAUDE.md", "AGENTS.md", "GEMINI.md"];
    let mut found = Vec::new();
    for name in &rule_filenames {
        if cron_tasks_dir.join(name).is_file() {
            found.push(name.to_string());
        }
    }
    if found.is_empty() {
        vec!["CLAUDE.md".to_string()]
    } else {
        found
    }
}
