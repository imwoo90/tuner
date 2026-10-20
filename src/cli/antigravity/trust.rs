//! # Antigravity Workspace Auto-Trust and Model Discovery
//!
//! ## Overview
//! Handles updating the `settings.json` file for Google Antigravity CLI to automatically
//! register `trustedWorkspaces`, as well as dynamic model discovery via `agy models`.
//!
//! ## Search Tags
//! #trust, #discovery, #workspace-trust, #model-catalog

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;
use tokio::time::timeout;

fn load_settings_json(settings_file: &Path) -> serde_json::Value {
    let mut data = if settings_file.is_file() {
        let content = fs::read_to_string(settings_file).unwrap_or_default();
        serde_json::from_str(&content)
            .unwrap_or_else(|_| serde_json::Value::Object(serde_json::Map::new()))
    } else {
        serde_json::Value::Object(serde_json::Map::new())
    };
    if !data.is_object() {
        data = serde_json::Value::Object(serde_json::Map::new());
    }
    data
}

/// Ensure that `workspace` (fully resolved path) is present in `trustedWorkspaces`
/// within settings.json (which resides under `.gemini/antigravity-cli/settings.json`).
///
/// If `home_override` is specified, it uses that directory as HOME; otherwise,
/// it resolves HOME from environment variables.
pub fn trust_workspace_in_settings(workspace: &Path, home_override: Option<PathBuf>) {
    let home = home_override.unwrap_or_else(|| {
        let home_str = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home_str)
    });

    let parent_dir = home.join(".gemini").join("antigravity-cli");
    if !parent_dir.is_dir() {
        return;
    }

    let settings_file = parent_dir.join("settings.json");
    let mut data = load_settings_json(&settings_file);

    let Some(obj) = data.as_object_mut() else {
        return;
    };
    let workspaces_val = obj
        .entry("trustedWorkspaces")
        .or_insert_with(|| serde_json::Value::Array(Vec::new()));

    if !workspaces_val.is_array() {
        *workspaces_val = serde_json::Value::Array(Vec::new());
    }

    let Some(workspaces) = workspaces_val.as_array_mut() else {
        return;
    };

    let ws_resolved = match workspace.canonicalize() {
        Ok(canon) => canon.to_string_lossy().to_string(),
        Err(_) => workspace.to_string_lossy().to_string(),
    };

    let exists = workspaces
        .iter()
        .any(|v| v.as_str().map(|s| s == ws_resolved).unwrap_or(false));

    if !exists {
        workspaces.push(serde_json::Value::String(ws_resolved));
        if let Ok(serialized) = serde_json::to_string_pretty(&data) {
            let _ = fs::write(settings_file, serialized);
        }
    }
}

/// Parse `agy models` stdout into a vector of model display names.
pub fn parse_models(output: &str) -> Vec<String> {
    let mut models = Vec::new();
    for line in output.lines() {
        let name = line.trim();
        if name.is_empty() {
            continue;
        }
        // A usage/help banner means the command was rejected - treat as failure.
        if name.starts_with("Usage:")
            || name.starts_with("Flags:")
            || name.starts_with("Available subcommands:")
        {
            return Vec::new();
        }
        if let Some(token) = name.split_whitespace().next() {
            models.push(token.to_string());
        }
    }
    models
}

/// Execute the `agy models` command and return discovered model display names.
///
/// Returns an empty vector on timeout, command missing, or error.
pub async fn discover_models(cmd_name: &str) -> Vec<String> {
    let mut cmd = Command::new(cmd_name);
    cmd.arg("models")
        .env_remove("CODEX_SANDBOX_NETWORK_DISABLED")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if std::env::var_os("SSH_CONNECTION").is_none() {
        cmd.env("SSH_CONNECTION", "tuner-headless");
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    // wait() takes &mut self, preserving ownership of child
    let wait_res = timeout(Duration::from_secs(15), child.wait()).await;
    match wait_res {
        Ok(Ok(status)) if status.success() => {
            let mut output = String::new();
            if let Some(mut stdout) = child.stdout.take() {
                use tokio::io::AsyncReadExt;
                let _ = stdout.read_to_string(&mut output).await;
            }
            parse_models(&output)
        }
        _ => {
            let _ = child.kill().await;
            Vec::new()
        }
    }
}
