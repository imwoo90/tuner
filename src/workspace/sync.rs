//! # Workspace Folder Synchronizer
//!
//! ## Overview
//! Initializes workspace layout folders (`cron_tasks`, `telegram_files`, `skills`) and mounts rules profiles.
//!
//! ## Search Tags
//! #workspace-init, #profile-copy, #environment-setup

use crate::workspace::paths::TunerPaths;
use crate::workspace::sync_helpers::{
    create_workspace_directories, migrate_legacy_data, smart_merge_config,
    smart_merge_profile_config, sync_constraints_rule, sync_mainmemory_rule, walk_and_copy,
};
use std::path::Path;

/// Initializes the workspace directory structure and configurations.
pub fn init_workspace(paths: &TunerPaths) -> Result<(), String> {
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

    let _ = crate::workspace::skills::sync_bundled_skills(paths);

    if paths.home_defaults.is_dir() {
        walk_and_copy(&paths.home_defaults, &paths.profile_home(), &paths.home_defaults)?;
    }

    create_workspace_directories(paths);
    let _ = sync_mainmemory_rule(&paths.workspace());
    let default_constraints = paths
        .home_defaults
        .join("workspace")
        .join(".agents")
        .join("rules")
        .join("constraints.md");
    let _ = sync_constraints_rule(&paths.workspace(), Some(&default_constraints));

    let selector = crate::workspace::rules::RulesSelector::new(paths.clone());
    let _ = selector.deploy_rules();

    let _ = smart_merge_profile_config(paths);
    let _ = sync_rule_files(&paths.workspace());

    if paths.workspace().is_dir()
        && let Ok(entries) = std::fs::read_dir(paths.workspace())
    {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_symlink() && !path.exists() {
                let _ = std::fs::remove_file(&path);
            }
        }
    }

    let _ = crate::workspace::skills::sync_skills(paths);
    Ok(())
}

/// Synchronizes persistent Antigravity native rule files (`mainmemory.md` and `constraints.md`) in the profile workspace.
pub fn sync_rule_files(root: &Path) -> Result<(), String> {
    if !root.is_dir() {
        return Ok(());
    }
    let _ = sync_mainmemory_rule(root);
    let _ = sync_constraints_rule(root, None);
    Ok(())
}
