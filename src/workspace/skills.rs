//! # Skill Directory Synchronization
//!
//! ## Overview
//! Synchronizes custom agent skill folders between tuner and CLI providers.
//! Discovers, validates, and symlinks skill directories across workspace and provider configuration folders.
//!
//! ## Search Tags
//! #skills

use crate::workspace::paths::DuctorPaths;
use crate::workspace::skills_helpers::{
    clean_broken_links, clean_cross_profile_skill_links, clean_invalid_workspace_skill_links,
    cli_skill_dirs, create_dir_link, ensure_copy, load_skill_sync_config,
    sync_profile_agent_skills,
};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Configuration options for skill synchronization.
#[derive(Clone, Debug)]
pub struct SkillConfig {
    /// Sync enabled.
    pub sync_enabled: bool,
    /// Sync providers.
    pub sync_providers: HashSet<String>,
}

/// Synchronizes custom and bundled skills strictly within the profile workspace (`<workspace>/.agents/skills`).
pub fn sync_skills(paths: &DuctorPaths, docker_active: bool) -> Result<(), String> {
    let (sync_enabled, _enabled_providers) = load_skill_sync_config(&paths.config_path());
    if !sync_enabled {
        return Ok(());
    }

    let _ = cleanup_ductor_links(paths);
    let _ = clean_cross_profile_skill_links(
        &paths.skills_dir(),
        &paths.profile_home(),
        &paths.tuner_home,
        &paths.bundled_skills_dir(),
    );
    let _ = clean_invalid_workspace_skill_links(&paths.skills_dir());
    let _ = clean_broken_links(&paths.skills_dir());

    let discovered = discover_skills(&paths.skills_dir());
    sync_profile_agent_skills(&paths.agents_skills_dir(), &discovered, docker_active)
        .map_err(|e| format!("Failed to sync profile agent skills: {}", e))?;

    Ok(())
}

/// Synchronizes bundled framework templates into the user's workspace skills directory.
pub fn sync_bundled_skills(paths: &DuctorPaths, docker_active: bool) -> Result<(), String> {
    let bundled = paths.bundled_skills_dir();
    if !bundled.is_dir() {
        return Ok(());
    }
    let target_dir = paths.skills_dir().join("builtin");
    let _ = std::fs::create_dir_all(&target_dir);

    let skip_dirs = [".claude", ".system", ".git", ".venv", "__pycache__", "node_modules"];
    if let Ok(entries) = std::fs::read_dir(bundled) {
        for entry in entries.flatten() {
          let path = entry.path();
          let name = match path.file_name().and_then(|s| s.to_str()) {
              Some(n) => n,
              None => continue,
          };
          if !path.is_dir() || name.starts_with('.') || skip_dirs.contains(&name) {
              continue;
          }
          if !has_valid_skill_frontmatter(&path) {
              continue;
          }
          let target = target_dir.join(name);
          if docker_active {
              let _ = ensure_copy(&target, &path);
          } else {
              if target.exists() && !target.is_symlink() {
                  continue;
              }
              if target.is_symlink() {
                  if let Ok(res) = std::fs::canonicalize(&target)
                      && let Ok(src_canon) = std::fs::canonicalize(&path)
                          && res == src_canon {
                              continue;
                          }
                    let _ = std::fs::remove_file(&target);
                }
                let _ = create_dir_link(&target, &path);
            }
        }
      }
      Ok(())
}

/// Cleans up tuner-created symlinks and copies from provider directories.
pub fn cleanup_ductor_links(paths: &DuctorPaths) -> Result<usize, String> {
    let mut managed_roots = Vec::new();
    for root in [paths.skills_dir(), paths.bundled_skills_dir()] {
        if let Ok(canon) = std::fs::canonicalize(&root) {
            managed_roots.push(canon);
        } else {
            managed_roots.push(root);
        }
    }

    let mut removed = 0;
    let providers = vec!["claude".to_string(), "codex".to_string(), "gemini".to_string()].into_iter().collect();
    let cli_dirs = cli_skill_dirs(&providers);

    for cli_dir in cli_dirs.values() {
        if !cli_dir.is_dir() {
            continue;
        }
        if let Ok(entries) = std::fs::read_dir(cli_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_symlink()
                    && let Ok(resolved) = std::fs::canonicalize(&path)
                        && managed_roots.iter().any(|r| resolved.starts_with(r))
                            && std::fs::remove_file(&path).is_ok() {
                                removed += 1;
                            }
            }
        }
    }
    Ok(removed)
}

/// Asynchronously monitors skill folders and triggers synchronization.
pub async fn watch_skill_sync(paths: &DuctorPaths, interval_ms: u64) -> Result<(), String> {
    let mut interval = tokio::time::interval(tokio::time::Duration::from_millis(interval_ms));
    loop {
        interval.tick().await;
        let paths_clone = paths.clone();
        let _ = tokio::task::spawn_blocking(move || {
            let _ = sync_skills(&paths_clone, false);
        }).await;
    }
}

fn scan_skill_entries(
    dir: &Path,
    skip_dirs: &[&str],
    overwrite: bool,
    skills: &mut HashMap<String, PathBuf>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if name.starts_with('.') || skip_dirs.contains(&name) {
            continue;
        }
        let is_dir = path.is_dir() || (path.is_symlink() && path.exists());
        if is_dir && has_valid_skill_frontmatter(&path) {
            if overwrite {
                skills.insert(name.to_string(), path);
            } else {
                skills.entry(name.to_string()).or_insert(path);
            }
        }
    }
}

/// Discovers skill subdirectories containing valid `SKILL.md` frontmatter.
pub fn discover_skills(dir: &Path) -> HashMap<String, PathBuf> {
    let mut skills = HashMap::new();
    if !dir.is_dir() {
        return skills;
    }
    let skip_dirs = [
        ".claude",
        ".system",
        ".git",
        ".venv",
        "__pycache__",
        "node_modules",
        "builtin",
        "archive",
    ];

    scan_skill_entries(dir, &skip_dirs, true, &mut skills);

    let builtin_dir = dir.join("builtin");
    if builtin_dir.is_dir() {
        scan_skill_entries(&builtin_dir, &skip_dirs, false, &mut skills);
    }

    skills
}

/// Verifies whether the skill directory contains a valid `SKILL.md` with YAML frontmatter.
pub fn has_valid_skill_frontmatter(skill_dir: &Path) -> bool {
    let skill_md = skill_dir.join("SKILL.md");
    let content = match std::fs::read_to_string(&skill_md) {
        Ok(c) => c,
        Err(_) => return false,
    };
    let lines: Vec<&str> = content.lines().collect();
    if lines.is_empty() || lines[0].trim() != "---" {
        return false;
    }
    let mut end_index = -1;
    for (i, line) in lines.iter().enumerate().skip(1) {
        if line.trim() == "---" {
            end_index = i as i32;
            break;
        }
    }
    if end_index <= 1 {
        return false;
    }

    let mut name_val = None;
    let mut desc_val = None;
    for line in &lines[1..end_index as usize] {
        let trimmed = line.trim();
        if let Some(part) = trimmed.strip_prefix("name:") {
            let clean = part.trim().trim_matches(|c| c == '\'' || c == '"').to_string();
            name_val = Some(clean);
        } else if let Some(part) = trimmed.strip_prefix("description:") {
            let clean = part.trim().trim_matches(|c| c == '\'' || c == '"').to_string();
            desc_val = Some(clean);
        }
    }

    match (name_val, desc_val) {
        (Some(n), Some(d)) => !n.trim().is_empty() && !d.trim().is_empty(),
        _ => false,
    }
}

/// Resolves the canonical path for duplicate skills using provider precedence.
pub fn resolve_canonical(
    name: &str,
    ductor: &HashMap<String, PathBuf>,
    claude: &HashMap<String, PathBuf>,
    codex: &HashMap<String, PathBuf>,
    gemini: &HashMap<String, PathBuf>,
) -> Option<PathBuf> {
    let registries = [ductor, claude, codex, gemini];
    for reg in &registries {
        if let Some(entry) = reg.get(name)
            && !entry.is_symlink() {
                return Some(entry.clone());
            }
    }
    for reg in &registries {
        if let Some(entry) = reg.get(name)
            && entry.is_symlink() && entry.exists()
                && let Ok(resolved) = std::fs::canonicalize(entry) {
                    return Some(resolved);
                }
    }
    None
}
