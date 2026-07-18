//! # Antigravity Workspace Auto-Trust Helper
//!
//! This module handles updating the `settings.json` file for Google Antigravity CLI,
//! automatically adding new workspaces to the list of `trustedWorkspaces`.

use std::fs;
use std::path::{Path, PathBuf};

/// Ensure that `workspace` (fully resolved path) is present in `trustedWorkspaces`
/// within settings.json (which resides under `.gemini/antigravity-cli/settings.json`).
///
/// If `home_override` is specified, it uses that directory as HOME; otherwise,
/// it resolves HOME from environment variables.
pub fn trust_workspace_in_settings(workspace: &Path, home_override: Option<PathBuf>) {
    let home = match home_override {
        Some(path) => path,
        None => {
            let home_str = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            PathBuf::from(home_str)
        }
    };

    let parent_dir = home.join(".gemini").join("antigravity-cli");
    if !parent_dir.is_dir() {
        return;
    }

    let settings_file = parent_dir.join("settings.json");
    let mut data: serde_json::Value = if settings_file.is_file() {
        let content = fs::read_to_string(&settings_file).unwrap_or_default();
        serde_json::from_str(&content).unwrap_or(serde_json::Value::Object(serde_json::Map::new()))
    } else {
        serde_json::Value::Object(serde_json::Map::new())
    };

    if !data.is_object() {
        data = serde_json::Value::Object(serde_json::Map::new());
    }

    let obj = data.as_object_mut().unwrap();
    let workspaces_val = obj
        .entry("trustedWorkspaces")
        .or_insert_with(|| serde_json::Value::Array(Vec::new()));

    if !workspaces_val.is_array() {
        *workspaces_val = serde_json::Value::Array(Vec::new());
    }

    let workspaces = workspaces_val.as_array_mut().unwrap();
    
    let ws_resolved = match workspace.canonicalize() {
        Ok(canon) => canon.to_string_lossy().to_string(),
        Err(_) => workspace.to_string_lossy().to_string(),
    };

    let exists = workspaces.iter().any(|v| {
        v.as_str().map(|s| s == ws_resolved).unwrap_or(false)
    });

    if !exists {
        workspaces.push(serde_json::Value::String(ws_resolved));
        if let Ok(serialized) = serde_json::to_string_pretty(&data) {
            let _ = fs::write(settings_file, serialized);
        }
    }
}
