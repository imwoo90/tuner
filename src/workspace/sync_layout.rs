//! # Workspace Layout and Rule Initialization
//!
//! Provides layout creation, legacy data migration, and persistent rule initialization
//! including mainmemory and operational constraints for the Tuner workspace environment.
//!
//! ## Search Tags
//! #workspace-layout, #rules-init, #constraints-sync

use crate::workspace::paths::DuctorPaths;
use std::path::Path;

pub fn create_workspace_directories(paths: &DuctorPaths) {
    let required_workspace_dirs = [
        "",
        "memory_system",
        "cron_tasks",
        "tools",
        "tools/user_tools",
        "tools/cron_tools",
        "tools/media_tools",
        "tools/webhook_tools",
        "output_to_user",
        "tasks",
        "skills",
        ".agents",
        ".agents/rules",
    ];
    for rel in &required_workspace_dirs {
        let d = if rel.is_empty() {
            paths.workspace()
        } else {
            paths.workspace().join(rel)
        };
        if !d.is_dir() {
            let _ = std::fs::create_dir_all(&d);
        }
    }
    let _ = std::fs::create_dir_all(paths.config_dir());
}

pub fn migrate_legacy_data(paths: &DuctorPaths) {
    if let Some(ref p) = paths.profile {
        if p == "default" {
            let legacy_sessions = paths.tuner_home.join("sessions.json");
            let legacy_workspace = paths.tuner_home.join("workspace");
            let target_sessions = paths.sessions_path();
            let target_workspace = paths.workspace();

            if legacy_sessions.is_file() && !target_sessions.exists() {
                if let Some(parent) = target_sessions.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::rename(&legacy_sessions, &target_sessions);
            }
            if legacy_workspace.is_dir() && !target_workspace.exists() {
                if let Some(parent) = target_workspace.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::rename(&legacy_workspace, &target_workspace);
            }
        }
    }
}

pub fn sync_mainmemory_rule(root: &Path) -> Result<(), String> {
    let rules_dir = root.join(".agents").join("rules");
    let target = rules_dir.join("mainmemory.md");
    let legacy_dir = root.join("memory_system");
    let legacy_mem = legacy_dir.join("MAINMEMORY.md");

    // 1. Migration: if legacy regular file exists and target doesn't exist, migrate it
    if legacy_mem.is_file() && !legacy_mem.is_symlink() && !target.is_file() {
        if let Ok(content) = std::fs::read_to_string(&legacy_mem) {
            let header = "---\ntrigger: always_on\ndescription: \"Core factual memory about user, family, assets, vehicle, and preferences\"\n---\n";
            let clean = if content.starts_with("---") {
                content
            } else {
                format!("{}{}", header, content)
            };
            let _ = std::fs::create_dir_all(&rules_dir);
            let _ = std::fs::write(&target, clean);
        }
    }

    // 2. Ensure legacy path is a relative symlink to target if legacy_dir exists or target exists
    if target.is_file() && legacy_dir.is_dir() {
        let rel_target = std::path::Path::new("../.agents/rules/mainmemory.md");
        let should_symlink = match std::fs::read_link(&legacy_mem) {
            Ok(p) => p != rel_target,
            Err(_) => true,
        };
        if should_symlink {
            let _ = std::fs::remove_file(&legacy_mem);
            #[cfg(unix)]
            {
                let _ = std::os::unix::fs::symlink(rel_target, &legacy_mem);
            }
        }
    }
    Ok(())
}

pub fn sync_constraints_rule(root: &Path, default_template: Option<&Path>) -> Result<(), String> {
    let rules_dir = root.join(".agents").join("rules");
    let target = rules_dir.join("constraints.md");

    // Only create if it doesn't already exist! Never overwrite user constraints.
    if !target.is_file() {
        let _ = std::fs::create_dir_all(&rules_dir);
        if let Some(tpl) = default_template.filter(|p| p.is_file()) {
            let _ = std::fs::copy(tpl, &target);
        } else {
            let content = "---\ntrigger: always_on\ndescription: \"User-defined negative and positive operational constraints\"\n---\n# Operational Constraints\n\n## Negative Constraints (절대 금지 사항)\n\n## Positive Constraints (필수 준수 표준)\n";
            let _ = std::fs::write(&target, content);
        }
    }
    Ok(())
}
