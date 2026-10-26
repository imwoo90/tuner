//! # Custom Skills Directory Link Resolver
//!
//! ## Overview
//! Provides utilities to index, parse YAML metadata fronts, create symlinks, and synchronize
//! skill directories in the workspace.
//!
//! ## Search Tags
//! #skills-helpers

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Returns whether skill synchronization is enabled in `config.json` (defaults to `true`).
pub fn is_skill_sync_enabled(config_path: &Path) -> bool {
    if let Ok(content) = std::fs::read_to_string(config_path)
        && let Ok(val) = serde_json::from_str::<serde_json::Value>(&content)
        && let Some(skills) = val.get("skills").and_then(|v| v.as_object())
        && let Some(sync_enabled) = skills.get("sync_enabled").and_then(|v| v.as_bool())
    {
        return sync_enabled;
    }
    true
}

#[cfg(unix)]
/// Executes `create_dir_link` (create dir link).
pub fn create_dir_link(link_path: &Path, target: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link_path)
}

#[cfg(not(unix))]
pub fn create_dir_link(link_path: &Path, target: &Path) -> std::io::Result<()> {
    if std::os::windows::fs::symlink_dir(target, link_path).is_ok() {
        return Ok(());
    }
    let status = std::process::Command::new("cmd")
        .args(&["/c", "mklink", "/J", link_path.to_str().unwrap(), target.to_str().unwrap()])
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(std::io::Error::new(std::io::ErrorKind::Other, "NTFS junction creation failed"))
    }
}

/// Executes `ensure_link` (ensure link).
pub fn ensure_link(link_path: &Path, target: &Path) -> std::io::Result<bool> {
    if link_path.exists() && !link_path.is_symlink() {
        return Ok(false);
    }
    if link_path.is_symlink() {
        if let Ok(resolved) = std::fs::canonicalize(link_path)
            && let Ok(target_canon) = std::fs::canonicalize(target)
                && resolved == target_canon {
                    return Ok(false);
                }
        let _ = std::fs::remove_file(link_path);
    }
    create_dir_link(link_path, target)?;
    Ok(true)
}

/// Executes `clean_broken_links` (clean broken links).
pub fn clean_broken_links(directory: &Path) -> usize {
    if !directory.is_dir() { return 0; }
    let mut removed = 0;
    if let Ok(entries) = std::fs::read_dir(directory) {
        for entry in entries.flatten() {
          let path = entry.path();
          if path.is_symlink() && !path.exists()
              && std::fs::remove_file(&path).is_ok() {
                  removed += 1;
                }
        }
      }
      removed
}

/// Executes `clean_invalid_workspace_skill_links` (clean invalid workspace skill links).
pub fn clean_invalid_workspace_skill_links(base_dir: &Path) -> usize {
    if !base_dir.is_dir() { return 0; }
    let mut removed = 0;
    let skip_dirs = [".system", ".git", ".venv", "__pycache__", "node_modules", "builtin", "archive"];
    if let Ok(entries) = std::fs::read_dir(base_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let name = match path.file_name().and_then(|s| s.to_str()) {
                Some(n) => n,
                None => continue,
            };
            if name.starts_with('.') || skip_dirs.contains(&name) {
                continue;
            }
            if path.is_symlink()
                && path.exists()
                && !crate::workspace::skills::has_valid_skill_frontmatter(&path)
                && std::fs::remove_file(&path).is_ok()
            {
                removed += 1;
            }
        }
    }
    removed
}

/// Removes symlinks in a profile's `skills/` directory that point to a sibling profile's workspace.
pub fn clean_cross_profile_skill_links(
    skills_dir: &Path,
    profile_home: &Path,
    tuner_home: &Path,
    bundled_dir: &Path,
) -> usize {
    if !skills_dir.is_dir() {
        return 0;
    }
    let Ok(profiles_canon) = std::fs::canonicalize(tuner_home.join("profiles")) else {
        return 0;
    };
    let Ok(prof_canon) = std::fs::canonicalize(profile_home) else {
        return 0;
    };
    let bundled_canon = std::fs::canonicalize(bundled_dir).ok();
    let mut removed = 0;
    if let Ok(entries) = std::fs::read_dir(skills_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_symlink()
                && let Ok(resolved) = std::fs::canonicalize(&path)
            {
                let is_bundled = bundled_canon
                    .as_ref()
                    .is_some_and(|b| resolved.starts_with(b));
                if resolved.starts_with(&profiles_canon)
                    && !resolved.starts_with(&prof_canon)
                    && !is_bundled
                    && std::fs::remove_file(&path).is_ok()
                {
                    removed += 1;
                }
            }
        }
    }
    removed
}

/// Synchronizes discovered profile skills into `<workspace>/.agents/skills/<name>` and prunes stale links.
pub fn sync_profile_agent_skills(
    agents_skills_dir: &Path,
    discovered: &HashMap<String, PathBuf>,
) -> std::io::Result<()> {
    std::fs::create_dir_all(agents_skills_dir)?;
    let mut names: Vec<&String> = discovered.keys().collect();
    names.sort();

    for name in names {
        if let Some(skill_path) = discovered.get(name) {
            let canonical = std::fs::canonicalize(skill_path).unwrap_or_else(|_| skill_path.clone());
            let dest = agents_skills_dir.join(name);
            let _ = ensure_link(&dest, &canonical);
        }
    }

    let _ = clean_broken_links(agents_skills_dir);
    if let Ok(entries) = std::fs::read_dir(agents_skills_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            if !discovered.contains_key(name) && path.is_symlink() {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
    Ok(())
}
