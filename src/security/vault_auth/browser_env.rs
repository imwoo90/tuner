//! # Browser Executable Discovery & Profile Lock Hygiene (`browser_env.rs`)
//!
//! ## Overview
//! Resolves the canonical desktop Chrome/Chromium binary (preserving identical TLS/WebGL
//! fingerprints across host and Docker Playwright environments) and safely cleans stale
//! `SingletonLock` artifacts from persistent Vault browser profile directories.
//!
//! ## Search Tags
//! #browser-discovery, #profile-lock-cleanup, #docker-sandbox-flags

use super::proxy::set_mode_0700;
use crate::setup::doctor::which_bin;
use std::path::{Path, PathBuf};

/// Resolves the Chrome/Chromium executable in deterministic fingerprint-preserving order.
pub fn resolve_browser_binary() -> Result<PathBuf, String> {
    if let Ok(env_bin) = std::env::var("TUNER_CHROME_BIN") {
        let p = PathBuf::from(env_bin.trim());
        if p.is_file() {
            return Ok(p);
        }
    }
    for sys_bin in [
        "/usr/bin/google-chrome",
        "/usr/bin/google-chrome-stable",
        "/usr/bin/chromium",
        "/usr/bin/chromium-browser",
    ] {
        let p = PathBuf::from(sys_bin);
        if p.is_file() {
            return Ok(p);
        }
    }
    for name in [
        "google-chrome",
        "google-chrome-stable",
        "chromium",
        "chromium-browser",
    ] {
        if let Some(p) = which_bin(name) {
            return Ok(p);
        }
    }
    if let Some(pw_bin) = find_playwright_chromium() {
        return Ok(pw_bin);
    }
    Err(
        "No Chrome or Chromium binary found. Install google-chrome or run `tuner --doctor --fix`."
            .to_string(),
    )
}

fn find_playwright_chromium() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    let cache_dir = PathBuf::from(home).join(".cache/ms-playwright");
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(cache_dir)
        .ok()?
        .filter_map(|e| e.ok().map(|d| d.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("chromium-"))
        })
        .collect();
    dirs.sort();
    dirs.reverse();
    for dir in dirs {
        for sub in ["chrome-linux64/chrome", "chrome-linux/chrome"] {
            let bin = dir.join(sub);
            if bin.is_file() {
                return Some(bin);
            }
        }
    }
    None
}

fn is_profile_in_use(profile_dir: &Path) -> bool {
    let needle = format!("--user-data-dir={}", profile_dir.display());
    let self_pid = std::process::id().to_string();
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return false;
    };
    for entry in entries.flatten() {
        let pid_name = entry.file_name();
        if pid_name.to_str() == Some(self_pid.as_str()) {
            continue;
        }
        if let Ok(bytes) = std::fs::read(entry.path().join("cmdline")) {
            let text = String::from_utf8_lossy(&bytes);
            if text.split('\0').any(|arg| arg == needle) {
                return true;
            }
        }
    }
    false
}

/// Removes stale `Singleton*` and `DevToolsActivePort` files if no live Chrome owns `profile_dir`.
pub fn prepare_profile_dir(profile_dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(profile_dir)
        .map_err(|e| format!("Failed to create browser profile dir {:?}: {}", profile_dir, e))?;
    set_mode_0700(profile_dir);
    let _ = std::fs::create_dir_all(profile_dir.join(".xdg_cache"));
    if is_profile_in_use(profile_dir) {
        return Err(format!(
            "Browser profile '{}' is already in use by another live Chrome session.",
            profile_dir.display()
        ));
    }
    for name in [
        "SingletonLock",
        "SingletonCookie",
        "SingletonSocket",
        "DevToolsActivePort",
    ] {
        let _ = std::fs::remove_file(profile_dir.join(name));
    }
    Ok(())
}

/// Returns `true` when Chrome must launch with `--no-sandbox` (root UID, container, or explicit env).
pub fn needs_no_sandbox() -> bool {
    if std::env::var("TUNER_NO_SANDBOX")
        .ok()
        .is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"))
    {
        return true;
    }
    if Path::new("/.dockerenv").exists() || Path::new("/run/.containerenv").exists() {
        return true;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if std::fs::metadata("/proc/self").is_ok_and(|m| m.uid() == 0) {
            return true;
        }
    }
    false
}
