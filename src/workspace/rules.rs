//! # Workspace Rule Profile Selector
//!
//! ## Overview
//! Deploys `AGENTS.md` configurations to the profile workspace and cleans up legacy rule files.
//!
//! ## Search Tags
//! #rule-profiles, #markdown-rules, #profile-mapping

use crate::workspace::paths::TunerPaths;
use std::path::{Path, PathBuf};

/// Selector and deployer for workspace `AGENTS.md` rule files.
pub struct RulesSelector {
    /// Paths.
    pub paths: TunerPaths,
}

impl RulesSelector {
    /// Returns a RulesSelector configured for the Antigravity-native Tuner environment.
    pub fn new(paths: TunerPaths) -> Self {
        Self { paths }
    }

    /// Selects the rule template file (`RULES.md` or `AGENTS.md`) from a directory.
    pub fn get_best_template(&self, directory: &Path) -> Option<PathBuf> {
        let static_template = directory.join("RULES.md");
        if static_template.is_file() {
            return Some(static_template);
        }
        let agents_template = directory.join("AGENTS.md");
        if agents_template.is_file() {
            return Some(agents_template);
        }
        None
    }

    /// Deploys `AGENTS.md` to the workspace root and cleans up stale rule files.
    pub fn deploy_rules(&self) -> Result<(), String> {
        let ws_template_dir = self.paths.home_defaults.join("workspace");
        let template_opt = self
            .get_best_template(&ws_template_dir)
            .or_else(|| self.get_best_template(&self.paths.home_defaults));

        if let Some(template) = template_opt {
            let dst_dir = self.paths.workspace();
            if let Err(e) = std::fs::create_dir_all(&dst_dir) {
                return Err(format!("Failed to create directory {:?}: {}", dst_dir, e));
            }
            let _ = std::fs::copy(&template, dst_dir.join("AGENTS.md"));
        }
        let _ = self.cleanup_stale_files();
        Ok(())
    }

    /// Purges legacy rule files and deprecated directories from managed profile folders.
    pub fn cleanup_stale_files(&self) -> Result<usize, String> {
        let mut total_removed = 0;
        for filename in &["CLAUDE.md", "GEMINI.md"] {
            total_removed += self.remove_files_by_name(filename)?;
        }

        let ws = self.paths.workspace();
        for dir in managed_profile_rule_dirs(&self.paths) {
            if dir == ws {
                continue;
            }
            let file_path = dir.join("AGENTS.md");
            if file_path.is_file() && std::fs::remove_file(&file_path).is_ok() {
                total_removed += 1;
            }
        }

        for legacy_dir in &[
            ws.join("tasks"),
            ws.join("tools").join("agent_tools"),
            ws.join("tools").join("task_tools"),
            ws.join("tools").join("webhook_tools"),
        ] {
            if legacy_dir.is_dir() && !legacy_dir.is_symlink() {
                let _ = std::fs::remove_dir_all(legacy_dir);
            }
        }
        let legacy_webhooks = self.paths.profile_home().join("webhooks.json");
        if legacy_webhooks.is_file() {
            let _ = std::fs::remove_file(&legacy_webhooks);
        }

        Ok(total_removed)
    }

    /// Removes files with the given name only from explicitly whitelisted profile-managed directories,
    /// never touching `cron_tasks/*`, `projects/`, `scratch/`, or other user directories.
    pub fn remove_files_by_name(&self, filename: &str) -> Result<usize, String> {
        let mut count = 0;
        for dir in managed_profile_rule_dirs(&self.paths) {
            let file_path = dir.join(filename);
            if file_path.is_file() && std::fs::remove_file(&file_path).is_ok() {
                count += 1;
            }
        }
        Ok(count)
    }
}

fn managed_profile_rule_dirs(paths: &TunerPaths) -> Vec<PathBuf> {
    use crate::workspace::sync_helpers::MANAGED_SINGLE_DIRS;
    let profile_home = paths.profile_home();
    let ws = paths.workspace();
    let mut dirs = vec![
        profile_home.clone(),
        profile_home.join("config"),
        ws.clone(),
    ];
    for d in MANAGED_SINGLE_DIRS {
        dirs.push(ws.join(d));
    }
    dirs.push(ws.join("cron_tasks"));
    for container in &["tools", "skills"] {
        let c_path = ws.join(container);
        dirs.push(c_path.clone());
        if let Ok(entries) = std::fs::read_dir(&c_path) {
            for entry in entries.flatten() {
                let child = entry.path();
                if child.is_dir() && !child.is_symlink() {
                    dirs.push(child);
                }
            }
        }
    }
    dirs
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_cleanup_stale_files_protects_projects_dir() {
        let tmp = tempdir().unwrap();
        let home = tmp.path().join("home");
        let paths = TunerPaths::new(
            home,
            tmp.path().join("defaults"),
            Some("default".to_string()),
        );
        let profile_ws = paths.workspace();
        let proj_dir = profile_ws.join("projects").join("my_proj");
        let cfg_dir = paths.profile_home().join("config");
        std::fs::create_dir_all(&proj_dir).unwrap();
        std::fs::create_dir_all(&cfg_dir).unwrap();

        let ws_gemini = profile_ws.join("GEMINI.md");
        let cfg_gemini = cfg_dir.join("GEMINI.md");
        let proj_gemini = proj_dir.join("GEMINI.md");

        std::fs::write(&ws_gemini, "ws").unwrap();
        std::fs::write(&cfg_gemini, "cfg").unwrap();
        std::fs::write(&proj_gemini, "proj").unwrap();

        let selector = RulesSelector::new(paths);
        let removed = selector.remove_files_by_name("GEMINI.md").unwrap();

        assert_eq!(removed, 2);
        assert!(!ws_gemini.exists());
        assert!(!cfg_gemini.exists());
        assert!(proj_gemini.exists());
    }
}
