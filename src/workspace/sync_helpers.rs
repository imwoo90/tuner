//! # Filesystem Sync Helpers
//!
//! ## Overview
//! Internal implementation helpers for `walk_and_copy` and non-destructive configuration merging.
//! Provides recursive directory copying and Zone 2 Python script backup handling.
//!
//! ## Search Tags
//! #sync-helpers

use crate::workspace::paths::TunerPaths;
use std::path::Path;

/// Executes `is_zone2_py_file` (is zone2 py file).
pub fn is_zone2_py_file(entry: &Path, src: &Path, root_src: &Path) -> bool {
    if entry.extension().and_then(|s| s.to_str()) != Some("py") {
        return false;
    }
    if let Ok(rel_dir) = src.strip_prefix(root_src) {
        let rel_str = rel_dir.to_string_lossy().replace("\\", "/");
        let zone2_py_dirs = [
            "workspace/tools",
            "workspace/tools/cron_tools",
            "workspace/tools/media_tools",
            "workspace/skills/memory-system/scripts",
            "workspace/skills/skill-creator/scripts",
        ];
        return zone2_py_dirs.contains(&rel_str.as_str());
    }
    false
}

fn is_bundled_system_doc(name: &str, src: &Path, root_src: &Path) -> bool {
    if let Ok(rel_dir) = src.strip_prefix(root_src) {
        let rel_str = rel_dir.to_string_lossy().replace("\\", "/");
        if name == "SKILL.md"
            && (rel_str == "workspace/skills/memory-system"
                || rel_str == "workspace/skills/skill-creator")
        {
            return true;
        }
        if name == "TASK_DESCRIPTION.md" && rel_str == "workspace/cron_tasks/memory-consolidator" {
            return true;
        }
    }
    false
}

/// Executes `walk_and_copy` (walk and copy).
pub fn walk_and_copy(src: &Path, dst: &Path, root_src: &Path) -> Result<(), String> {
    if let Err(e) = std::fs::create_dir_all(dst) {
        return Err(format!("Failed to create dir: {}", e));
    }
    let skip_dirs = [".venv", ".git", ".mypy_cache", "__pycache__", "node_modules"];

    let entries = match std::fs::read_dir(src) {
        Ok(e) => e,
        Err(e) => return Err(format!("Failed to read dir: {}", e)),
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let name = match path.file_name().and_then(|s| s.to_str()) {
            Some(n) => n,
            None => continue,
        };
        if name.starts_with('.') || skip_dirs.contains(&name) {
            continue;
        }
        if name == "RULES.md" && (src == root_src || src == root_src.join("workspace")) {
            continue;
        }
        let target = dst.join(name);
        handle_entry(&path, &target, name, src, root_src)?;
    }
    Ok(())
}

fn handle_entry(path: &Path, target: &Path, name: &str, src: &Path, root_src: &Path) -> Result<(), String> {
    if path.is_dir() {
        if !target.is_symlink() {
            walk_and_copy(path, target, root_src)?;
        }
    } else if name == "RULES.md" || name == "AGENTS.md" || is_bundled_system_doc(name, src, root_src) {
        if target.is_symlink() || target.is_file() {
            let _ = std::fs::remove_file(target);
        }
        let _ = std::fs::copy(path, target);
    } else if is_zone2_py_file(path, src, root_src) {
        if target.is_file() {
            let same_content = match (std::fs::read(path), std::fs::read(target)) {
                (Ok(a), Ok(b)) => a == b,
                _ => false,
            };
            if !same_content {
                let backup = target.with_extension("py.bak");
                let _ = std::fs::rename(target, &backup);
            }
        }
        if target.is_symlink() {
            let _ = std::fs::remove_file(target);
        }
        let _ = std::fs::copy(path, target);
    } else if !target.exists() {
        let _ = std::fs::copy(path, target);
    }
    Ok(())
}

/// Explicitly whitelisted single-level system directories inside a profile workspace.
pub const MANAGED_SINGLE_DIRS: &[&str] = &[
    "memory_system",
    "output_to_user",
    "telegram_files",
];

/// Executes `smart_merge_config` (smart merge config).
pub fn smart_merge_config(paths: &TunerPaths) -> Result<(), String> {
    let example_path = paths.config_example_path();
    if !example_path.is_file() {
        return Ok(());
    }
    let defaults_content = match std::fs::read_to_string(&example_path) {
        Ok(c) => c,
        Err(_) => return Ok(()),
    };
    let defaults: serde_json::Value = match serde_json::from_str(&defaults_content) {
        Ok(v) => v,
        Err(_) => return Ok(()),
    };

    let config_path = paths.config_path();
    if !config_path.is_file() {
        if let Some(parent) = config_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(pretty) = serde_json::to_string_pretty(&defaults) {
            let _ = std::fs::write(&config_path, pretty);
        }
    }
    Ok(())
}

/// Executes `smart_merge_profile_config` (smart merge profile config).
pub fn smart_merge_profile_config(paths: &TunerPaths) -> Result<(), String> {
    let example_path = paths.home_defaults.join("config").join("config.json");
    if !example_path.is_file() {
        return Ok(());
    }
    let defaults_content = std::fs::read_to_string(&example_path).map_err(|e| e.to_string())?;
    let defaults: serde_json::Value = serde_json::from_str(&defaults_content).map_err(|e| e.to_string())?;

    let config_path = paths.profile_home().join("config").join("config.json");
    if !config_path.is_file() {
        if let Some(parent) = config_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(pretty) = serde_json::to_string_pretty(&defaults) {
            let _ = std::fs::write(&config_path, pretty);
        }
    }
    Ok(())
}

pub use crate::workspace::sync_layout::{
    create_workspace_directories, migrate_legacy_data, sync_constraints_rule, sync_mainmemory_rule,
};
