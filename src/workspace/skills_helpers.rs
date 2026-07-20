//! # Custom Skills Directory Link Resolver
//!
//! Provides utilities to index, parse YAML metadata fronts, create symlinks, and synchronize
//! skill directories in the workspace.

//! 
//! ## Search Tags
//! #skills-helpers

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

pub fn load_skill_sync_config(config_path: &Path) -> (bool, HashSet<String>) {
    let mut providers: HashSet<String> = vec!["claude".to_string(), "codex".to_string(), "gemini".to_string()].into_iter().collect();
    let mut enabled = true;
    if let Ok(content) = std::fs::read_to_string(config_path) {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
            if let Some(skills) = val.get("skills").and_then(|v| v.as_object()) {
                if let Some(sync_enabled) = skills.get("sync_enabled").and_then(|v| v.as_bool()) {
                    enabled = sync_enabled;
                }
                if let Some(sync) = skills.get("sync").and_then(|v| v.as_object()) {
                    let mut new_providers = HashSet::new();
                    for p in &["claude", "codex", "gemini"] {
                        if sync.get(*p).and_then(|v| v.as_bool()).unwrap_or(true) {
                            new_providers.insert(p.to_string());
                        }
                    }
                    providers = new_providers;
                }
            }
        }
    }
    (enabled, providers)
}

pub fn is_under(child: &Path, parent: &Path) -> bool {
    let child = match child.canonicalize() {
        Ok(p) => p,
        Err(_) => return false,
    };
    let parent = match parent.canonicalize() {
        Ok(p) => p,
        Err(_) => return false,
    };
    child.starts_with(parent)
}

pub fn cli_skill_dirs(enabled_providers: &HashSet<String>) -> HashMap<String, PathBuf> {
    let mut dirs = HashMap::new();
    let home_str = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let home = Path::new(&home_str);

    if enabled_providers.contains("claude") {
        let claude_home = home.join(".claude");
        if claude_home.is_dir() {
            dirs.insert("claude".to_string(), claude_home.join("skills"));
        }
    }
    if enabled_providers.contains("codex") {
        let codex_home = std::env::var("CODEX_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| home.join(".codex"));
        if codex_home.is_dir() {
            dirs.insert("codex".to_string(), codex_home.join("skills"));
        }
    }
    if enabled_providers.contains("gemini") {
        let gemini_home = home.join(".gemini");
        if gemini_home.is_dir() {
            dirs.insert("gemini".to_string(), gemini_home.join("skills"));
        }
    }
    dirs
}

#[cfg(unix)]
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

pub fn ensure_link(link_path: &Path, target: &Path) -> std::io::Result<bool> {
    if link_path.exists() && !link_path.is_symlink() {
        return Ok(false);
    }
    if link_path.is_symlink() {
        if let Ok(resolved) = std::fs::canonicalize(link_path) {
            if let Ok(target_canon) = std::fs::canonicalize(target) {
                if resolved == target_canon {
                    return Ok(false);
                }
            }
        }
        let _ = std::fs::remove_file(link_path);
    }
    create_dir_link(link_path, target)?;
    Ok(true)
}

pub fn ensure_copy(dest: &Path, source: &Path) -> std::io::Result<bool> {
    let marker = dest.join(".tuner_managed");
    if is_managed_copy(dest) {
        if let (Ok(src_time), Ok(dest_meta)) = (newest_mtime(source), std::fs::metadata(&marker)) {
            if let Ok(dest_time) = dest_meta.modified() {
                if src_time <= dest_time {
                    return Ok(false);
                }
            }
        }
        let _ = std::fs::remove_dir_all(dest);
    } else if dest.exists() && !dest.is_symlink() {
        return Ok(false);
    }
    if dest.is_symlink() {
        let _ = std::fs::remove_file(dest);
    }
    copy_dir_all(source, dest)?;
    let _ = std::fs::File::create(&marker);
    Ok(true)
}

pub fn is_managed_copy(path: &Path) -> bool {
    path.is_dir() && !path.is_symlink() && path.join(".tuner_managed").is_file()
}

pub fn newest_mtime(dir: &Path) -> std::io::Result<std::time::SystemTime> {
    let mut newest = std::fs::metadata(dir)?.modified()?;
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries {
            if let Ok(entry) = entry {
                let path = entry.path();
                if path.is_dir() {
                    if let Ok(t) = newest_mtime(&path) {
                        newest = newest.max(t);
                    }
                } else if let Ok(meta) = std::fs::metadata(&path) {
                    if let Ok(t) = meta.modified() {
                        newest = newest.max(t);
                    }
                }
            }
        }
    }
    Ok(newest)
}

pub fn copy_dir_all(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        if path.is_dir() {
            copy_dir_all(&path, &dst.join(name))?;
        } else {
            std::fs::copy(&path, &dst.join(name))?;
        }
    }
    Ok(())
}

pub fn clean_broken_links(directory: &Path) -> usize {
    if !directory.is_dir() { return 0; }
    let mut removed = 0;
    if let Ok(entries) = std::fs::read_dir(directory) {
        for entry in entries {
            if let Ok(entry) = entry {
                let path = entry.path();
                if path.is_symlink() && !path.exists() {
                    if std::fs::remove_file(&path).is_ok() {
                        removed += 1;
                      }
                  }
              }
          }
      }
      removed
}

pub fn clean_invalid_workspace_skill_links(base_dir: &Path) -> usize {
    if !base_dir.is_dir() { return 0; }
    let mut removed = 0;
    let skip_dirs = [".claude", ".system", ".git", ".venv", "__pycache__", "node_modules"];
    if let Ok(entries) = std::fs::read_dir(base_dir) {
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
                if path.is_symlink() && path.exists() && !crate::workspace::skills::has_valid_skill_frontmatter(&path) {
                    if std::fs::remove_file(&path).is_ok() {
                        removed += 1;
                    }
                } else if is_managed_copy(&path) && !crate::workspace::skills::has_valid_skill_frontmatter(&path) {
                    if std::fs::remove_dir_all(&path).is_ok() {
                        removed += 1;
                    }
                }
            }
        }
    }
    removed
}

pub fn link_skill_everywhere(skill_name: &str, canonical: &Path, all_dirs: &HashMap<String, PathBuf>, docker_active: bool) -> std::io::Result<()> {
    let mut sync_roots = HashSet::new();
    for d in all_dirs.values() {
        if d.is_dir() {
            if let Ok(canon) = std::fs::canonicalize(d) {
                sync_roots.insert(canon);
            }
        }
    }

    let canon_resolved = std::fs::canonicalize(canonical)?;

    for base_dir in all_dirs.values() {
        if !base_dir.is_dir() {
            let _ = std::fs::create_dir_all(base_dir);
        }
        let dest = base_dir.join(skill_name);
        if let Ok(dest_canon) = std::fs::canonicalize(&dest) {
            if dest_canon == canon_resolved {
                continue;
            }
        }

        let skip = if docker_active {
            dest.exists() && !dest.is_symlink() && !is_managed_copy(&dest)
        } else {
            if dest.exists() && !dest.is_symlink() {
                true
            } else if dest.is_symlink() && dest.exists() {
                if let Ok(res) = std::fs::canonicalize(&dest) {
                    !sync_roots.iter().any(|r| res.starts_with(r))
                } else {
                    false
                }
            } else {
                false
            }
        };

        if skip {
            continue;
        }

        if docker_active {
            let _ = ensure_copy(&dest, canonical);
        } else {
            let _ = ensure_link(&dest, canonical);
        }
    }
    Ok(())
}
