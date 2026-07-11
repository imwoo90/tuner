//! # Antigravity Model Discovery
//!
//! This module implements dynamic discovery of models from the `agy models` CLI command.
//! It executes the CLI with a timeout and parses the stdout stream.

use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;
use tokio::time::timeout;

/// Parse `agy models` stdout into a vector of model display names.
pub fn parse_models(output: &str) -> Vec<String> {
    let mut models = Vec::new();
    for line in output.lines() {
        let name = line.trim();
        if name.is_empty() {
            continue;
        }
        // A usage/help banner means the command was rejected - treat as failure.
        if name.starts_with("Usage:") || name.starts_with("Flags:") || name.starts_with("Available subcommands:") {
            return Vec::new();
        }
        models.push(name.to_string());
    }
    models
}

/// Execute the `agy models` command and return discovered model display names.
///
/// Returns an empty vector on timeout, command missing, or error.
pub async fn discover_models(cmd_name: &str) -> Vec<String> {
    let mut child = match Command::new(cmd_name)
        .arg("models")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
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
