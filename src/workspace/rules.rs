//! # Workspace Rule Profile Selector
//!
//! ## Overview
//! Parses custom user profiles and determines corresponding CLAUDE.md/GEMINI.md/AGENTS.md configurations.
//!
//! ## Collaboration Graph
//! - Called by [`init_workspace`](super::sync::init_workspace) to sync rule configurations.
//!
//! ## Search Tags
//! #rule-profiles, #markdown-rules, #profile-mapping

use crate::workspace::paths::DuctorPaths;
use crate::workspace::rules_check::{check_antigravity, check_claude, check_codex, check_gemini};
use std::path::{Path, PathBuf};

/// Authentication status for CLI providers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthStatus {
    Authenticated,
    Installed,
    NotFound,
}

/// Verification result for a provider.
#[derive(Clone, Debug)]
pub struct AuthResult {
    pub provider: String,
    pub status: AuthStatus,
    pub auth_file: Option<PathBuf>,
    pub auth_age: Option<chrono::DateTime<chrono::Utc>>,
}

/// Selector and deployer for variant-specific rule files.
pub struct RulesSelector {
    pub paths: DuctorPaths,
    pub claude_authenticated: bool,
    pub codex_authenticated: bool,
    pub gemini_authenticated: bool,
    pub antigravity_authenticated: bool,
}

impl RulesSelector {
    /// Discovers auth status and returns a RulesSelector instance.
    pub fn new(paths: DuctorPaths) -> Self {
        let claude = check_claude(&paths);
        let codex = check_codex(&paths);
        let gemini = check_gemini(&paths);
        let agy = check_antigravity(&paths);
        Self {
            paths,
            claude_authenticated: claude.status == AuthStatus::Authenticated,
            codex_authenticated: codex.status == AuthStatus::Authenticated,
            gemini_authenticated: gemini.status == AuthStatus::Authenticated,
            antigravity_authenticated: agy.status == AuthStatus::Authenticated,
        }
    }

    /// Determines variant suffix (e.g. "claude-only", "all-clis", "gemini-only").
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

    /// Discovers directories in `home_defaults` containing templates.
    pub fn discover_template_directories(&self) -> Vec<PathBuf> {
        let mut candidates = Vec::new();
        find_rules_templates(&self.paths.home_defaults, &mut candidates);
        candidates.sort();
        candidates
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

    /// Deploys rule files to matching locations.
    pub fn deploy_rules(&self) -> Result<(), String> {
        let template_dirs = self.discover_template_directories();
        for template_dir in template_dirs {
            let rel_path = match template_dir.strip_prefix(&self.paths.home_defaults) {
                Ok(p) => p,
                Err(_) => continue,
            };
            let template = match self.get_best_template(&template_dir) {
                Some(t) => t,
                None => continue,
            };
            let dst_dir = self.paths.profile_home().join(rel_path);
            if let Err(e) = std::fs::create_dir_all(&dst_dir) {
                return Err(format!("Failed to create directory {:?}: {}", dst_dir, e));
            }
            if self.claude_authenticated {
                let _ = std::fs::copy(&template, dst_dir.join("CLAUDE.md"));
            }
            if self.codex_authenticated || self.antigravity_authenticated {
                let _ = std::fs::copy(&template, dst_dir.join("AGENTS.md"));
            }
            if self.gemini_authenticated {
                let _ = std::fs::copy(&template, dst_dir.join("GEMINI.md"));
            }
        }
        let _ = self.cleanup_stale_files();
        Ok(())
    }

    /// Removes rule files for unauthenticated CLI providers.
    pub fn cleanup_stale_files(&self) -> Result<usize, String> {
        let mut stale = Vec::new();
        if !self.claude_authenticated {
            stale.push("CLAUDE.md");
        }
        if !self.codex_authenticated && !self.antigravity_authenticated {
            stale.push("AGENTS.md");
        }
        if !self.gemini_authenticated {
            stale.push("GEMINI.md");
        }
        let mut total_removed = 0;
        for filename in stale {
            total_removed += self.remove_files_by_name(filename)?;
        }
        Ok(total_removed)
    }

    /// Removes files with the given name recursively, protecting cron task and project directories.
    pub fn remove_files_by_name(&self, filename: &str) -> Result<usize, String> {
        let cron_tasks_path = self.paths.workspace().join("cron_tasks");
        let projects_path = self.paths.workspace().join("projects");
        let mut count = 0;
        remove_files_recursively(
            &self.paths.profile_home(),
            filename,
            &cron_tasks_path,
            &projects_path,
            &mut count,
        );
        Ok(count)
    }
}

fn find_rules_templates(dir: &Path, candidates: &mut Vec<PathBuf>) {
    if !dir.is_dir() { return; }
    let mut has_rules = false;
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries {
            if let Ok(entry) = entry {
                let path = entry.path();
                if path.is_dir() {
                    find_rules_templates(&path, candidates);
                } else if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                    if name.starts_with("RULES") && name.ends_with(".md") {
                        has_rules = true;
                    }
                }
            }
        }
    }
    if has_rules {
        candidates.push(dir.to_path_buf());
    }
}

fn remove_files_recursively(
    dir: &Path,
    filename: &str,
    cron_tasks_path: &Path,
    projects_path: &Path,
    count: &mut usize,
) {
    if !dir.is_dir() { return; }
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if path.starts_with(cron_tasks_path) || path.starts_with(projects_path) {
                    continue;
                }
                if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                    if name.starts_with('.') || name == "node_modules" || name == "target" {
                        continue;
                    }
                }
                remove_files_recursively(&path, filename, cron_tasks_path, projects_path, count);
            } else if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                if name == filename && !path.starts_with(cron_tasks_path) && !path.starts_with(projects_path) {
                    if std::fs::remove_file(&path).is_ok() {
                        *count += 1;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
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
