//! # Workspace Rule Profile Selector
//!
//! ## Overview
//! Deploys `AGENTS.md` configurations to the profile workspace and cleans up legacy rule files.
//!
//! ## Search Tags
//! #rule-profiles, #markdown-rules, #profile-mapping

use crate::workspace::paths::DuctorPaths;
use std::path::{Path, PathBuf};

fn check_antigravity_installed(paths: &DuctorPaths) -> bool {
    if std::env::var("TUNER_TEST_MODE").is_ok() {
        return false;
    }
    let home = if paths.tuner_home.ends_with(".tuner") {
        paths.tuner_home.parent().unwrap_or(&paths.tuner_home).to_path_buf()
    } else {
        paths.tuner_home.clone()
    };
    home.join(".gemini").join("antigravity-cli").is_dir()
        || home.join(".local").join("bin").join("agy").exists()
        || home.join(".ccs").join("agy.settings.json").is_file()
}

/// Selector and deployer for variant-specific rule files.
pub struct RulesSelector {
    /// Paths.
    pub paths: DuctorPaths,
    /// Claude authenticated.
    pub claude_authenticated: bool,
    /// Codex authenticated.
    pub codex_authenticated: bool,
    /// Gemini authenticated.
    pub gemini_authenticated: bool,
    /// Antigravity authenticated.
    pub antigravity_authenticated: bool,
}

impl RulesSelector {
    /// Returns a RulesSelector configured for the Antigravity-native Tuner environment.
    pub fn new(paths: DuctorPaths) -> Self {
        let antigravity_authenticated = check_antigravity_installed(&paths);
        Self {
            paths,
            claude_authenticated: false,
            codex_authenticated: false,
            gemini_authenticated: false,
            antigravity_authenticated,
        }
    }

    /// Determines variant suffix (e.g. "antigravity", "claude-only").
    pub fn get_variant_suffix(&self) -> String {
        let auth_count = self.claude_authenticated as usize
            + self.codex_authenticated as usize
            + self.gemini_authenticated as usize
            + self.antigravity_authenticated as usize;

        if auth_count >= 2 {
            "all-clis".to_string()
        } else if self.codex_authenticated {
            "codex-only".to_string()
        } else if self.antigravity_authenticated {
            "antigravity".to_string()
        } else if self.gemini_authenticated {
            "gemini-only".to_string()
        } else {
            "claude-only".to_string()
        }
    }

    /// Selects the best template (variant or static) for a directory.
    pub fn get_best_template(&self, directory: &Path) -> Option<PathBuf> {
        let variant = self.get_variant_suffix();
        let variant_template = directory.join(format!("RULES-{}.md", variant));
        if variant_template.is_file() {
            return Some(variant_template);
        }
        if variant == "antigravity" {
            let gemini_template = directory.join("RULES-gemini-only.md");
            if gemini_template.is_file() {
                return Some(gemini_template);
            }
        }
        let static_template = directory.join("RULES.md");
        if static_template.is_file() {
            return Some(static_template);
        }
        None
    }

    /// Deploys rule files to the workspace root and cleans up stale/legacy rule files.
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
            if self.claude_authenticated {
                let _ = std::fs::copy(&template, dst_dir.join("CLAUDE.md"));
            }
            if self.codex_authenticated
                || self.antigravity_authenticated
                || (!self.claude_authenticated && !self.gemini_authenticated)
            {
                let _ = std::fs::copy(&template, dst_dir.join("AGENTS.md"));
            }
            if self.gemini_authenticated {
                let _ = std::fs::copy(&template, dst_dir.join("GEMINI.md"));
            }
        }
        let _ = self.cleanup_stale_files();
        Ok(())
    }

    /// Removes rule files for unauthenticated CLI providers and purges legacy subdirectory rule files.
    pub fn cleanup_stale_files(&self) -> Result<usize, String> {
        let mut stale = Vec::new();
        if !self.claude_authenticated {
            stale.push("CLAUDE.md");
        }
        if !self.gemini_authenticated {
            stale.push("GEMINI.md");
        }
        let mut total_removed = 0;
        for filename in stale {
            total_removed += self.remove_files_by_name(filename)?;
        }

        let ws = self.paths.workspace();
        for dir in managed_profile_rule_dirs(&self.paths) {
            if dir == ws {
                continue;
            }
            for filename in &["CLAUDE.md", "AGENTS.md", "GEMINI.md"] {
                let file_path = dir.join(filename);
                if file_path.is_file() && std::fs::remove_file(&file_path).is_ok() {
                    total_removed += 1;
                }
            }
        }

        for legacy_dir in &[
            ws.join("tasks"),
            ws.join("tools").join("agent_tools"),
            ws.join("tools").join("task_tools"),
        ] {
            if legacy_dir.is_dir() && !legacy_dir.is_symlink() {
                let _ = std::fs::remove_dir_all(legacy_dir);
            }
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

fn managed_profile_rule_dirs(paths: &DuctorPaths) -> Vec<PathBuf> {
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
    for container in &["tasks", "tools", "skills"] {
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
        let paths = DuctorPaths::new(
            home,
            tmp.path().join("defaults"),
            tmp.path().join("fw"),
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
