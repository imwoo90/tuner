//! # Background CLI Provider Installer (`provider_install.rs`)
//!
//! ## Overview
//! Runs non-root user-space CLI installation commands for `antigravity` (`agy`), `claude`,
//! and `codex`, capturing rolling stdout/stderr logs and broadcasting `ProviderSetupUpdated`
//! events to the Web Dashboard.
//!
//! ## Search Tags
//! #provider-installer, #cli-bootstrap, #onboarding-install

use super::events::WebEvent;
use super::provider_status::{current_home_dir, ensure_agy_headless_settings};
use super::WebAppCtx;
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, OnceLock};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::Mutex;

/// Live state of a background CLI installation job.
#[derive(Clone, Debug, Default, Serialize)]
pub struct InstallJobView {
    /// True while the installer process is still running.
    pub running: bool,
    /// Process exit code once completed (`0` on success).
    pub exit_code: Option<i32>,
    /// Rolling tail of merged stdout/stderr output.
    pub log_tail: String,
}

static INSTALL_JOBS: OnceLock<Mutex<HashMap<String, InstallJobView>>> = OnceLock::new();

fn jobs_map() -> &'static Mutex<HashMap<String, InstallJobView>> {
    INSTALL_JOBS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Returns the latest [`InstallJobView`] snapshot for `provider`, if any.
pub async fn get_install_job(provider: &str) -> Option<InstallJobView> {
    jobs_map().lock().await.get(provider).cloned()
}

fn build_install_shell_cmd(provider: &str) -> Result<String, String> {
    let env_key = format!("TUNER_CLI_INSTALL_CMD_{}", provider.to_ascii_uppercase());
    if let Ok(custom) = std::env::var(&env_key)
        && !custom.trim().is_empty()
    {
        return Ok(custom);
    }
    match provider {
        "antigravity" => Ok(
            "set -e; mkdir -p \"$HOME/.local/bin\" \"$HOME/.gemini/antigravity-cli/bin\"; \
             curl -fsSL https://antigravity.google/cli/install.sh | bash; \
             if [ -f \"$HOME/.local/bin/agy\" ] && [ ! -f \"$HOME/.gemini/antigravity-cli/bin/agy\" ]; then \
               ln -sf \"$HOME/.local/bin/agy\" \"$HOME/.gemini/antigravity-cli/bin/agy\"; \
             elif [ -f \"$HOME/.gemini/antigravity-cli/bin/agy\" ] && [ ! -f \"$HOME/.local/bin/agy\" ]; then \
               ln -sf \"$HOME/.gemini/antigravity-cli/bin/agy\" \"$HOME/.local/bin/agy\"; \
             fi"
                .to_string(),
        ),
        "claude" => Ok(
            "set -e; mkdir -p \"$HOME/.local/bin\"; \
             curl -fsSL https://claude.ai/install.sh | bash; \
             if [ -f \"$HOME/.claude/local/claude\" ] && [ ! -f \"$HOME/.local/bin/claude\" ]; then \
               ln -sf \"$HOME/.claude/local/claude\" \"$HOME/.local/bin/claude\"; \
             fi"
                .to_string(),
        ),
        "codex" => Ok(
            "set -e; mkdir -p \"$HOME/.local/bin\"; \
             npm install --prefix \"$HOME/.local\" -g @openai/codex"
                .to_string(),
        ),
        _ => Err(format!("Unsupported provider `{provider}`")),
    }
}

async fn append_install_line(provider: &str, line: &str) {
    let mut g = jobs_map().lock().await;
    let entry = g.entry(provider.to_string()).or_default();
    if !entry.log_tail.is_empty() {
        entry.log_tail.push('\n');
    }
    entry.log_tail.push_str(line);
    if entry.log_tail.len() > 8192 {
        let keep_from = entry.log_tail.len().saturating_sub(6144);
        let aligned = entry.log_tail[keep_from..]
            .find('\n')
            .map(|i| keep_from + i + 1)
            .unwrap_or(keep_from);
        entry.log_tail = entry.log_tail[aligned..].to_string();
    }
}

fn run_post_install_hooks(provider: &str, home: &str, working_dir: &std::path::Path) {
    if provider == "antigravity" {
        ensure_agy_headless_settings(home, working_dir);
        if let Some(vault_bin) = crate::setup::find_agy_vault_bin(home) {
            let _ = std::process::Command::new(vault_bin)
                .arg("patch-binary")
                .status();
        }
    }
}

fn spawn_install_monitor(
    ctx: Arc<WebAppCtx>,
    prov: String,
    home: String,
    mut child: tokio::process::Child,
) {
    tokio::spawn(async move {
        if let Some(stdout) = child.stdout.take() {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                append_install_line(&prov, &line).await;
            }
        }
        let code = child.wait().await.ok().and_then(|s| s.code()).unwrap_or(-1);
        if code == 0 {
            let h = PathBuf::from(&home);
            run_post_install_hooks(&prov, &h.to_string_lossy(), &ctx.working_dir);
            append_install_line(&prov, "✓ Installation finished successfully.").await;
        } else {
            append_install_line(&prov, &format!("✗ Installer exited with status {code}.")).await;
        }
        {
            let mut g = jobs_map().lock().await;
            if let Some(entry) = g.get_mut(&prov) {
                entry.running = false;
                entry.exit_code = Some(code);
            }
        }
        ctx.events
            .publish(WebEvent::ProviderSetupUpdated { provider: prov })
            .await;
    });
}

/// Starts a background CLI installation job for `provider`.
pub async fn start_provider_install(ctx: Arc<WebAppCtx>, provider: &str) -> Result<(), String> {
    let shell_cmd = build_install_shell_cmd(provider)?;
    {
        let mut g = jobs_map().lock().await;
        if g.get(provider).is_some_and(|j| j.running) {
            return Err(format!("Installation for `{provider}` is already in progress"));
        }
        g.insert(
            provider.to_string(),
            InstallJobView {
                running: true,
                exit_code: None,
                log_tail: format!("$ Installing {provider} CLI..."),
            },
        );
    }

    let prov = provider.to_string();
    let home = current_home_dir();
    let path_env = format!(
        "{}/.gemini/antigravity-cli/bin:{}/.local/bin:{}",
        home,
        home,
        std::env::var("PATH").unwrap_or_default()
    );
    let child = Command::new("bash")
        .arg("-c")
        .arg(format!("({shell_cmd}) 2>&1"))
        .env("HOME", &home)
        .env("PATH", path_env)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Failed to spawn installer: {e}"))?;

    ctx.events
        .publish(WebEvent::ProviderSetupUpdated { provider: prov.clone() })
        .await;
    spawn_install_monitor(ctx, prov, home, child);
    Ok(())
}
