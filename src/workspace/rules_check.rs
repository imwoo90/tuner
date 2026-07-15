//! # Rules Authentication Checkers
//!
//! Helper functions to check authentication status of different providers.

use crate::workspace::paths::DuctorPaths;
use crate::workspace::rules::{AuthResult, AuthStatus};
use std::path::{Path, PathBuf};

fn get_mtime(p: &Path) -> Option<chrono::DateTime<chrono::Utc>> {
    std::fs::metadata(p).and_then(|m| m.modified()).map(chrono::DateTime::from).ok()
}

pub fn check_claude(paths: &DuctorPaths) -> AuthResult {
    let home = get_home_dir(paths);
    let claude_dir = home.join(".claude");
    let credentials = claude_dir.join(".credentials.json");

    if credentials.is_file() {
        return AuthResult {
            provider: "claude".to_string(),
            status: AuthStatus::Authenticated,
            auth_file: Some(credentials.clone()),
            auth_age: get_mtime(&credentials),
        };
    }
    if std::env::var("ANTHROPIC_API_KEY").map(|s| !s.trim().is_empty()).unwrap_or(false) {
        return AuthResult {
            provider: "claude".to_string(),
            status: AuthStatus::Authenticated,
            auth_file: None,
            auth_age: None,
        };
    }
    if check_claude_cli_logged_in() {
        return AuthResult {
            provider: "claude".to_string(),
            status: AuthStatus::Authenticated,
            auth_file: None,
            auth_age: None,
        };
    }
    let status = if claude_dir.is_dir() { AuthStatus::Installed } else { AuthStatus::NotFound };
    AuthResult { provider: "claude".to_string(), status, auth_file: None, auth_age: None }
}

pub fn check_codex(paths: &DuctorPaths) -> AuthResult {
    let home = get_home_dir(paths);
    let codex_home = std::env::var("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home.join(".codex"));
    let auth_file = codex_home.join("auth.json");

    if auth_file.is_file() {
        return AuthResult {
            provider: "codex".to_string(),
            status: AuthStatus::Authenticated,
            auth_file: Some(auth_file.clone()),
            auth_age: get_mtime(&auth_file),
        };
    }
    if std::env::var("OPENAI_API_KEY").map(|s| !s.trim().is_empty()).unwrap_or(false) {
        return AuthResult {
            provider: "codex".to_string(),
            status: AuthStatus::Authenticated,
            auth_file: None,
            auth_age: None,
        };
    }
    let status = if codex_home.join("version.json").is_file() || codex_home.join("config.toml").is_file() {
        AuthStatus::Installed
    } else {
        AuthStatus::NotFound
    };
    AuthResult { provider: "codex".to_string(), status, auth_file: None, auth_age: None }
}

pub fn check_gemini(paths: &DuctorPaths) -> AuthResult {
    if !find_gemini_cli() {
        return AuthResult { provider: "gemini".to_string(), status: AuthStatus::NotFound, auth_file: None, auth_age: None };
    }
    let home = get_home_dir(paths);
    let gemini_home = std::env::var("GEMINI_CLI_HOME")
        .map(|s| PathBuf::from(s).join(".gemini"))
        .unwrap_or_else(|_| home.join(".gemini"));

    let oauth_file = gemini_home.join("oauth_creds.json");
    if oauth_file.is_file() && std::fs::metadata(&oauth_file).map(|m| m.len() > 0).unwrap_or(false) {
        return AuthResult { provider: "gemini".to_string(), status: AuthStatus::Authenticated, auth_file: Some(oauth_file.clone()), auth_age: get_mtime(&oauth_file) };
    }
    let env_path = gemini_home.join(".env");
    let parent_env_path = gemini_home.parent().map(|p| p.join(".env")).unwrap_or_else(|| PathBuf::from("/nonexistent"));
    let has_env = std::env::var("GEMINI_API_KEY").map(|s| !s.trim().is_empty()).unwrap_or(false)
        || std::env::var("GOOGLE_API_KEY").map(|s| !s.trim().is_empty()).unwrap_or(false)
        || (std::env::var("GOOGLE_CLOUD_PROJECT").is_ok() && std::env::var("GOOGLE_CLOUD_LOCATION").is_ok());
    
    if has_env || read_dotenv_has_gemini_keys(&env_path) || read_dotenv_has_gemini_keys(&parent_env_path) || config_has_gemini_key(paths) {
        return AuthResult { provider: "gemini".to_string(), status: AuthStatus::Authenticated, auth_file: None, auth_age: None };
    }
    if let Some(status) = gemini_settings_auth(&gemini_home) {
        return AuthResult { provider: "gemini".to_string(), status, auth_file: None, auth_age: None };
    }
    AuthResult { provider: "gemini".to_string(), status: AuthStatus::Installed, auth_file: None, auth_age: None }
}

pub fn check_antigravity(paths: &DuctorPaths) -> AuthResult {
    if std::env::var("TUNER_TEST_MODE").is_ok() {
        return AuthResult { provider: "antigravity".to_string(), status: AuthStatus::NotFound, auth_file: None, auth_age: None };
    }
    let binary_found = std::process::Command::new("agy").arg("models").output().is_ok();
    if binary_found && check_antigravity_cli_logged_in() {
        return AuthResult { provider: "antigravity".to_string(), status: AuthStatus::Authenticated, auth_file: None, auth_age: None };
    }
    let home = get_home_dir(paths);
    let ccs_settings = home.join(".ccs").join("agy.settings.json");
    let status = if binary_found || ccs_settings.is_file() { AuthStatus::Installed } else { AuthStatus::NotFound };
    AuthResult { provider: "antigravity".to_string(), status, auth_file: None, auth_age: None }
}

fn get_home_dir(paths: &DuctorPaths) -> PathBuf {
    if paths.tuner_home.ends_with(".tuner") {
        paths.tuner_home.parent().unwrap_or(&paths.tuner_home).to_path_buf()
    } else {
        paths.tuner_home.clone()
    }
}

fn check_claude_cli_logged_in() -> bool {
    if std::env::var("TUNER_TEST_MODE").is_ok() {
        return false;
    }
    match std::process::Command::new("claude").args(&["auth", "status"]).output() {
        Ok(output) if output.status.success() => {
            if let Ok(stdout_str) = std::str::from_utf8(&output.stdout) {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(stdout_str) {
                    return val.get("loggedIn") == Some(&serde_json::Value::Bool(true));
                }
            }
            false
        }
        _ => false,
    }
}

fn find_gemini_cli() -> bool {
    if std::env::var("TUNER_TEST_MODE").is_ok() {
        return false;
    }
    if std::process::Command::new("gemini").arg("--version").output().is_ok() {
        return true;
    }
    if let Some(home_str) = std::env::var("HOME").ok() {
        let home = Path::new(&home_str);
        let versions_dir = home.join(".nvm").join("versions").join("node");
        if versions_dir.is_dir() {
            if let Ok(entries) = std::fs::read_dir(versions_dir) {
                for entry in entries {
                    if let Ok(entry) = entry {
                        if entry.path().join("bin").join("gemini").is_file() {
                            return true;
                        }
                    }
                }
            }
        }
    }
    false
}

fn read_dotenv_has_gemini_keys(path: &Path) -> bool {
    if let Ok(content) = std::fs::read_to_string(path) {
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') { continue; }
            let clean = if trimmed.starts_with("export ") { trimmed["export ".len()..].trim() } else { trimmed };
            if let Some(idx) = clean.find('=') {
                let key = clean[..idx].trim();
                let value = clean[idx + 1..].trim();
                if (key == "GEMINI_API_KEY" || key == "GOOGLE_API_KEY") && !value.is_empty() && value != "\"\"" && value != "''" {
                    return true;
                }
            }
        }
    }
    false
}

fn config_has_gemini_key(paths: &DuctorPaths) -> bool {
    let path = paths.config_path();
    if let Ok(content) = std::fs::read_to_string(&path) {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
            if let Some(key) = val.get("gemini_api_key").and_then(|v| v.as_str()) {
                return !key.trim().is_empty();
            }
        }
    }
    false
}

fn gemini_settings_auth(gemini_home: &Path) -> Option<AuthStatus> {
    let settings_file = gemini_home.join("settings.json");
    if let Ok(content) = std::fs::read_to_string(&settings_file) {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
            let selected_type = val.pointer("/security/auth/selectedType").and_then(|v| v.as_str());
            if let Some(sel) = selected_type {
                if sel == "oauth-personal" {
                    let accounts_file = gemini_home.join("google_accounts.json");
                    if let Ok(accs_content) = std::fs::read_to_string(&accounts_file) {
                        if let Ok(accs_val) = serde_json::from_str::<serde_json::Value>(&accs_content) {
                            if let Some(active) = accs_val.get("active").and_then(|v| v.as_str()) {
                                if !active.trim().is_empty() {
                                    return Some(AuthStatus::Authenticated);
                                }
                            }
                        }
                    }
                } else if sel == "gemini-api-key" || sel == "vertex-ai" || sel == "compute-default-credentials" || sel == "cloud-shell" {
                    return Some(AuthStatus::Authenticated);
                }
            }
        }
    }
    None
}

fn check_antigravity_cli_logged_in() -> bool {
    match std::process::Command::new("agy").arg("models").output() {
        Ok(output) => {
            let merged = format!("{}\n{}", 
                std::str::from_utf8(&output.stdout).unwrap_or_default(),
                std::str::from_utf8(&output.stderr).unwrap_or_default()
            ).to_lowercase();
            if merged.contains("sign in") || merged.contains("not logged in") || merged.contains("login") {
                return false;
            }
            output.status.success()
        }
        _ => false,
    }
}
