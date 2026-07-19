//! # Filesystem Sync Helpers
//!
//! Internal implementation helpers for walk_and_copy, sync_group, and smart_merge.

use crate::workspace::paths::DuctorPaths;
use std::path::Path;

pub fn is_zone2_py_file(entry: &Path, src: &Path, root_src: &Path) -> bool {
    if entry.extension().and_then(|s| s.to_str()) != Some("py") {
        return false;
    }
    if let Ok(rel_dir) = src.strip_prefix(root_src) {
        let rel_str = rel_dir.to_string_lossy().replace("\\", "/");
        let zone2_py_dirs = [
            "workspace/tools/cron_tools",
            "workspace/tools/webhook_tools",
            "workspace/tools/agent_tools",
            "workspace/tools/task_tools",
            "workspace/tools/media_tools",
        ];
        return zone2_py_dirs.contains(&rel_str.as_str());
    }
    false
}

pub fn walk_and_copy(src: &Path, dst: &Path, root_src: &Path) -> Result<(), String> {
    if let Err(e) = std::fs::create_dir_all(dst) {
        return Err(format!("Failed to create dir: {}", e));
    }
    let skip_dirs = [".venv", ".git", ".mypy_cache", "__pycache__", "node_modules"];
    let skip_files = [
        "RULES-claude-only.md",
        "RULES-codex-only.md",
        "RULES-gemini-only.md",
        "RULES-all-clis.md",
        "RULES.md",
    ];

    let entries = match std::fs::read_dir(src) {
        Ok(e) => e,
        Err(e) => return Err(format!("Failed to read dir: {}", e)),
    };

    for entry in entries {
        if let Ok(entry) = entry {
            let path = entry.path();
            let name = match path.file_name().and_then(|s| s.to_str()) {
                Some(n) => n,
                None => continue,
            };
            if name.starts_with('.') || skip_dirs.contains(&name) || skip_files.contains(&name) {
                continue;
            }
            let target = dst.join(name);
            handle_entry(&path, &target, name, src, root_src, dst)?;
        }
    }
    Ok(())
}

fn handle_entry(path: &Path, target: &Path, name: &str, src: &Path, root_src: &Path, dst: &Path) -> Result<(), String> {
    if path.is_dir() {
        if !target.is_symlink() {
            walk_and_copy(path, target, root_src)?;
        }
    } else if name == "CLAUDE.md" || name == "AGENTS.md" || name == "GEMINI.md" {
        if target.is_symlink() || target.is_file() {
            let _ = std::fs::remove_file(target);
        }
        let _ = std::fs::copy(path, target);
        if name == "CLAUDE.md" {
            for mirror in &["AGENTS.md", "GEMINI.md"] {
                let mirror_target = dst.join(mirror);
                if mirror_target.is_symlink() || mirror_target.is_file() {
                    let _ = std::fs::remove_file(&mirror_target);
                }
                let _ = std::fs::copy(path, &mirror_target);
            }
        }
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

pub fn sync_rule_files_recursive(dir: &Path) -> Result<(), String> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => return Err(format!("Failed to read dir: {}", e)),
    };
    let skip_dirs = [".venv", ".git", ".mypy_cache", "__pycache__", "node_modules"];

    for entry in entries {
        if let Ok(entry) = entry {
            let path = entry.path();
            if path.is_dir() {
                if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                    if skip_dirs.contains(&name) || name.starts_with('.') {
                        continue;
                    }
                }
                sync_group(&path)?;
                sync_rule_files_recursive(&path)?;
            }
        }
    }
    Ok(())
}

pub fn sync_group(directory: &Path) -> Result<(), String> {
    let filenames = ["CLAUDE.md", "AGENTS.md", "GEMINI.md"];
    let mut existing = Vec::new();

    for name in &filenames {
        let path = directory.join(name);
        if path.is_file() {
            if let Ok(meta) = std::fs::metadata(&path) {
                if let Ok(mtime) = meta.modified() {
                    existing.push((*name, path, mtime));
                }
            }
        }
    }
    if existing.is_empty() {
        return Ok(());
    }

    let (newest_name, newest_path, newest_mtime) = existing
        .iter()
        .max_by_key(|(_, _, mtime)| mtime)
        .unwrap();

    for name in &filenames {
        if *name == *newest_name { continue; }
        let target_path = directory.join(name);
        if target_path.is_file() {
            let is_older = if let Ok(meta) = std::fs::metadata(&target_path) {
                if let Ok(mtime) = meta.modified() {
                    mtime < *newest_mtime
                } else { true }
            } else { true };

            if is_older {
                if let Err(e) = std::fs::copy(newest_path, &target_path) {
                    return Err(format!("Failed to copy rule file: {}", e));
                }
                let ft_newest = filetime::FileTime::from_last_modification_time(&std::fs::metadata(newest_path).unwrap());
                let _ = filetime::set_file_mtime(&target_path, ft_newest);
            }
        }
    }
    Ok(())
}

pub fn smart_merge_config(paths: &DuctorPaths) -> Result<(), String> {
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

pub fn smart_merge_profile_config(paths: &DuctorPaths) -> Result<(), String> {
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
