//! # Skill Directory Synchronization
//!
//! Synchronizes custom agent skill folders between tuner and CLI providers.

//! 
//! ## Search Tags
//! #skills

use crate::workspace::paths::DuctorPaths;
use crate::workspace::skills_helpers::{
    clean_broken_links, clean_invalid_workspace_skill_links, cli_skill_dirs, create_dir_link,
    ensure_copy, load_skill_sync_config,
    link_skill_everywhere,
};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Configuration options for skill synchronization.
#[derive(Clone, Debug)]
pub struct SkillConfig {
    pub sync_enabled: bool,
    pub sync_providers: HashSet<String>,
}

/// Synchronizes custom skills across the workspace and CLI directories.
pub fn sync_skills(paths: &DuctorPaths, docker_active: bool) -> Result<(), String> {
    let (sync_enabled, enabled_providers) = load_skill_sync_config(&paths.config_path());
    if !sync_enabled {
        return Ok(());
    }
    let cli_dirs = cli_skill_dirs(&enabled_providers);
    let mut all_dirs = HashMap::new();
    all_dirs.insert("ductor".to_string(), paths.skills_dir());
    for (k, v) in cli_dirs {
        all_dirs.insert(k, v);
    }

    let _ = clean_invalid_workspace_skill_links(&paths.skills_dir());

    let mut registries = HashMap::new();
    for (name, dir) in &all_dirs {
        registries.insert(name.clone(), discover_skills(dir));
    }

    let mut all_names = HashSet::new();
    for reg in registries.values() {
        for key in reg.keys() {
            all_names.insert(key.clone());
        }
    }

    let ductor_reg = registries.get("ductor").cloned().unwrap_or_default();
    let claude_reg = registries.get("claude").cloned().unwrap_or_default();
    let codex_reg = registries.get("codex").cloned().unwrap_or_default();
    let gemini_reg = registries.get("gemini").cloned().unwrap_or_default();

    let mut names: Vec<String> = all_names.into_iter().collect();
    names.sort();

    for name in names {
        if let Some(canonical) = resolve_canonical(&name, &ductor_reg, &claude_reg, &codex_reg, &gemini_reg) {
            let _ = link_skill_everywhere(&name, &canonical, &all_dirs, docker_active);
        }
    }

    for dir in all_dirs.values() {
        let _ = clean_broken_links(dir);
    }
    Ok(())
}

/// Synchronizes bundled framework templates into the user's workspace skills directory.
pub fn sync_bundled_skills(paths: &DuctorPaths, docker_active: bool) -> Result<(), String> {
    let bundled = paths.bundled_skills_dir();
    if !bundled.is_dir() {
        return Ok(());
    }
    let target_dir = paths.skills_dir();
    let _ = std::fs::create_dir_all(&target_dir);

    let skip_dirs = [".claude", ".system", ".git", ".venv", "__pycache__", "node_modules"];
    if let Ok(entries) = std::fs::read_dir(bundled) {
        for entry in entries {
            if let Ok(entry) = entry {
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
                        if let Ok(res) = std::fs::canonicalize(&target) {
                            if let Ok(src_canon) = std::fs::canonicalize(&path) {
                                if res == src_canon {
                                    continue;
                                }
                              }
                          }
                          let _ = std::fs::remove_file(&target);
                      }
                      let _ = create_dir_link(&target, &path);
                  }
              }
          }
      }
      Ok(())
}

/// Cleans up tuner-created symlinks and copies from provider directories.
pub fn cleanup_ductor_links(paths: &DuctorPaths) -> Result<usize, String> {
    let mut managed_roots = vec![paths.skills_dir()];
    let bundled = paths.bundled_skills_dir();
    if bundled.is_dir() {
        managed_roots.push(bundled);
    }

    let mut removed = 0;
    let providers = vec!["claude".to_string(), "codex".to_string(), "gemini".to_string()].into_iter().collect();
    let cli_dirs = cli_skill_dirs(&providers);

    for cli_dir in cli_dirs.values() {
        if !cli_dir.is_dir() {
            continue;
        }
        if let Ok(entries) = std::fs::read_dir(cli_dir) {
            for entry in entries {
                if let Ok(entry) = entry {
                    let path = entry.path();
                    if path.is_symlink() {
                        if let Ok(resolved) = std::fs::canonicalize(&path) {
                            if managed_roots.iter().any(|r| resolved.starts_with(r)) {
                                if std::fs::remove_file(&path).is_ok() {
                                    removed += 1;
                                }
                            }
                        }
                    }
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

/// Discovers skill subdirectories containing valid `SKILL.md` frontmatter.
pub fn discover_skills(dir: &Path) -> HashMap<String, PathBuf> {
    let mut skills = HashMap::new();
    if !dir.is_dir() {
        return skills;
    }
    let skip_dirs = [".claude", ".system", ".git", ".venv", "__pycache__", "node_modules"];
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries {
            if let Ok(entry) = entry {
                let path = entry.path();
                let name = match path.file_name().and_then(|s| s.to_str()) {
                    Some(n) => n,
                    None => continue,
                };
                if name.starts_with('.') || skip_dirs.contains(&name) {
                    continue;
                }
                let is_dir = path.is_dir() || (path.is_symlink() && path.exists());
                if is_dir && has_valid_skill_frontmatter(&path) {
                    skills.insert(name.to_string(), path);
                }
            }
        }
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
        if trimmed.starts_with("name:") {
            let part = trimmed["name:".len()..].trim();
            let clean = part.trim_matches(|c| c == '\'' || c == '"').to_string();
            name_val = Some(clean);
        } else if trimmed.starts_with("description:") {
            let part = trimmed["description:".len()..].trim();
            let clean = part.trim_matches(|c| c == '\'' || c == '"').to_string();
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
        if let Some(entry) = reg.get(name) {
            if !entry.is_symlink() {
                return Some(entry.clone());
            }
        }
    }
    for reg in &registries {
        if let Some(entry) = reg.get(name) {
            if entry.is_symlink() && entry.exists() {
                if let Ok(resolved) = std::fs::canonicalize(entry) {
                    return Some(resolved);
                }
            }
        }
    }
    None
}
