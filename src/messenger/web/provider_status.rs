//! # CLI Provider Readiness, `.env` Secrets & Headless Bootstrap (`provider_status.rs`)
//!
//! ## Overview
//! Inspects installation and authentication status across `antigravity` (`agy`), `claude`,
//! and `codex`, overlays runtime `$TUNER_HOME/.env` credentials onto peer CLI spawns,
//! and pre-seeds headless `~/.gemini/antigravity-cli/settings.json` trust defaults.
//!
//! ## Search Tags
//! #provider-readiness, #onboarding-status, #dotenv-secrets, #agy-settings

use crate::cli::antigravity::AntigravityCli;
use serde::Serialize;
use std::collections::HashMap;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Diagnostic snapshot of a single CLI provider (`antigravity`, `claude`, `codex`).
#[derive(Clone, Debug, Serialize)]
pub struct ProviderReadiness {
    /// Canonical provider ID (`antigravity`, `claude`, `codex`).
    pub id: &'static str,
    /// Human-readable display title.
    pub name: &'static str,
    /// True when the CLI executable is present on disk.
    pub installed: bool,
    /// True when OAuth credentials or a valid API key are configured.
    pub authenticated: bool,
    /// Detected CLI `--version` string, if installed.
    pub version: Option<String>,
    /// Resolved executable path, if installed.
    pub binary_path: Option<String>,
    /// Concise authentication summary (`OAuth signed in`, `API Key configured`, `Not signed in`).
    pub auth_detail: String,
}

/// Resolves the current `$HOME` directory.
pub fn current_home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| "/root".to_string())
}

/// Parses `KEY=VALUE` lines from `$TUNER_HOME/.env` without mutating process globals.
pub fn read_dotenv_map(env_path: &Path) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let Ok(raw) = std::fs::read_to_string(env_path) else {
        return map;
    };
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let body = trimmed.strip_prefix("export ").unwrap_or(trimmed);
        if let Some((k, v)) = body.split_once('=') {
            let key = k.trim();
            let val = v.trim().trim_matches('"').trim_matches('\'');
            if !key.is_empty() {
                map.insert(key.to_string(), val.to_string());
            }
        }
    }
    map
}

/// Builds child environment for `claude`/`codex`, overlaying `$TUNER_HOME/.env` secrets dynamically.
pub fn build_peer_env(cli: &AntigravityCli) -> HashMap<String, String> {
    let mut env = cli.build_env();
    if let Some(th) = env.get("TUNER_HOME").cloned() {
        let dotenv = read_dotenv_map(&PathBuf::from(th).join(".env"));
        for (k, v) in dotenv {
            if !v.is_empty() {
                env.insert(k, v);
            }
        }
    }
    env
}

/// Atomically sets or updates `key=value` inside `env_path` (`chmod 0600`).
pub fn upsert_dotenv_secret(env_path: &Path, key: &str, value: &str) -> Result<(), String> {
    if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err("Invalid environment variable key".to_string());
    }
    if value.contains('\n') || value.contains('\r') {
        return Err("Secret value must not contain newlines".to_string());
    }
    if let Some(parent) = env_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let existing = std::fs::read_to_string(env_path).unwrap_or_default();
    let mut lines: Vec<String> = Vec::new();
    let mut replaced = false;
    let prefix = format!("{key}=");
    let export_prefix = format!("export {key}=");
    for line in existing.lines() {
        let t = line.trim_start();
        if t.starts_with(&prefix) || t.starts_with(&export_prefix) {
            if !replaced {
                lines.push(format!("{key}={value}"));
                replaced = true;
            }
        } else {
            lines.push(line.to_string());
        }
    }
    if !replaced {
        lines.push(format!("{key}={value}"));
    }
    let mut out = lines.join("\n");
    out.push('\n');
    let tmp = env_path.with_extension("env.tmp");
    std::fs::write(&tmp, out).map_err(|e| e.to_string())?;
    let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    std::fs::rename(&tmp, env_path).map_err(|e| e.to_string())?;
    let _ = std::fs::set_permissions(env_path, std::fs::Permissions::from_mode(0o600));
    Ok(())
}

/// Ensures `~/.gemini/antigravity-cli/settings.json` skips interactive first-run TUI prompts.
pub fn ensure_agy_headless_settings(home: &str, working_dir: &Path) {
    let dir = PathBuf::from(home).join(".gemini/antigravity-cli");
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let settings_path = dir.join("settings.json");
    let mut obj = std::fs::read_to_string(&settings_path)
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .filter(|v| v.is_object())
        .unwrap_or_else(|| serde_json::json!({}));

    if let Some(map) = obj.as_object_mut() {
        map.insert("onboardingComplete".to_string(), serde_json::json!(true));
        map.entry("allowNonWorkspaceAccess".to_string())
            .or_insert_with(|| serde_json::json!(true));
        map.entry("toolPermission".to_string())
            .or_insert_with(|| serde_json::json!("always-proceed"));
        let ws_str = working_dir.to_string_lossy().to_string();
        let arr = map
            .entry("trustedWorkspaces".to_string())
            .or_insert_with(|| serde_json::json!([]));
        if let Some(vec) = arr.as_array_mut()
            && !ws_str.is_empty()
            && !vec.iter().any(|v| v.as_str() == Some(ws_str.as_str()))
        {
            vec.push(serde_json::json!(ws_str));
        }
    }
    if let Ok(serialized) = serde_json::to_string_pretty(&obj) {
        let _ = std::fs::write(&settings_path, serialized);
        let _ = std::fs::set_permissions(&settings_path, std::fs::Permissions::from_mode(0o600));
    }
}

/// Resolves the executable path for `provider` (`antigravity`, `claude`, `codex`).
pub fn find_cli_binary(home: &str, provider: &str) -> Option<PathBuf> {
    let bin_name = match provider {
        "antigravity" => "agy",
        "claude" => "claude",
        "codex" => "codex",
        _ => return None,
    };
    let mut candidates = vec![
        PathBuf::from(home).join(".gemini/antigravity-cli/bin").join(bin_name),
        PathBuf::from(home).join(".local/bin").join(bin_name),
        PathBuf::from(home).join(".claude/local").join(bin_name),
        PathBuf::from(home).join(".local/node-lts/bin").join(bin_name),
    ];
    if let Some(paths) = std::env::var_os("PATH") {
        for p in std::env::split_paths(&paths) {
            candidates.push(p.join(bin_name));
        }
    }
    candidates.into_iter().find(|p| p.is_file())
}

fn probe_version(path: &Path) -> Option<String> {
    let out = Command::new(path).arg("--version").output().ok()?;
    let raw = String::from_utf8_lossy(&out.stdout);
    let first = raw.lines().next().unwrap_or("").trim();
    if !first.is_empty() {
        Some(first.chars().take(64).collect())
    } else {
        Some("installed".to_string())
    }
}

fn non_empty_file(path: &Path, min_bytes: u64) -> bool {
    std::fs::metadata(path).map(|m| m.is_file() && m.len() >= min_bytes).unwrap_or(false)
}

fn has_env_secret(dotenv: &HashMap<String, String>, key: &str) -> bool {
    if let Some(v) = dotenv.get(key)
        && !v.trim().is_empty()
    {
        return true;
    }
    std::env::var(key).map(|v| !v.trim().is_empty()).unwrap_or(false)
}

fn inspect_auth_status(
    home: &str,
    dotenv: &HashMap<String, String>,
    provider: &str,
) -> (&'static str, &'static str, bool, String) {
    match provider {
        "antigravity" => {
            let tok = PathBuf::from(home).join(".gemini/antigravity-cli/antigravity-oauth-token");
            let ok = non_empty_file(&tok, 8);
            (
                "antigravity",
                "Google Antigravity (agy)",
                ok,
                if ok { "Google OAuth signed in".into() } else { "Not signed in".into() },
            )
        }
        "claude" => {
            let creds = PathBuf::from(home).join(".claude/.credentials.json");
            let oauth_ok = non_empty_file(&creds, 12);
            let key_ok = has_env_secret(dotenv, "ANTHROPIC_API_KEY");
            let detail = if oauth_ok {
                "Claude OAuth signed in"
            } else if key_ok {
                "ANTHROPIC_API_KEY configured"
            } else {
                "Not signed in"
            };
            ("claude", "Claude Code (claude)", oauth_ok || key_ok, detail.to_string())
        }
        _ => {
            let auth_json = PathBuf::from(home).join(".codex/auth.json");
            let oauth_ok = non_empty_file(&auth_json, 12);
            let key_ok = has_env_secret(dotenv, "OPENAI_API_KEY");
            let detail = if oauth_ok {
                "Codex auth.json configured"
            } else if key_ok {
                "OPENAI_API_KEY configured"
            } else {
                "Not signed in"
            };
            ("codex", "OpenAI Codex (codex)", oauth_ok || key_ok, detail.to_string())
        }
    }
}

/// Inspects installation and authentication status for a single provider.
pub fn inspect_provider(home: &str, tuner_home: &Path, provider: &str) -> ProviderReadiness {
    let dotenv = read_dotenv_map(&tuner_home.join(".env"));
    let bin_opt = find_cli_binary(home, provider);
    let version = bin_opt.as_deref().and_then(probe_version);
    let binary_path = bin_opt.as_ref().map(|p| p.display().to_string());
    let installed = bin_opt.is_some();
    let (id, name, authenticated, auth_detail) = inspect_auth_status(home, &dotenv, provider);

    ProviderReadiness {
        id,
        name,
        installed,
        authenticated,
        version,
        binary_path,
        auth_detail,
    }
}

/// Inspects all three supported CLI providers and returns `(any_ready, list)`.
pub fn inspect_all_providers(tuner_home: &Path) -> (bool, Vec<ProviderReadiness>) {
    let home = current_home_dir();
    let list = vec![
        inspect_provider(&home, tuner_home, "antigravity"),
        inspect_provider(&home, tuner_home, "claude"),
        inspect_provider(&home, tuner_home, "codex"),
    ];
    let any_ready = list.iter().any(|p| p.installed && p.authenticated);
    (any_ready, list)
}
