//! # Interactive CLI OAuth Login Bridge (`provider_oauth.rs`)
//!
//! ## Overview
//! Drives headless interactive CLI sign-in flows (`agy`, `claude auth login`, `codex login`)
//! over safe pseudo-terminals (`SessionHolder`), capturing both `BROWSER=` callback URLs
//! (for zero-paste Web-in-Web Chrome completion) and PKCE authorization-code paste prompts.
//!
//! ## Search Tags
//! #cli-oauth, #pty-login-bridge, #web-in-web-oauth, #onboarding-auth

use super::events::WebEvent;
use super::provider_status::{
    current_home_dir, ensure_agy_headless_settings, find_cli_binary, inspect_provider,
};
use super::WebAppCtx;
use crate::cli::antigravity::pty_spawner::{spawn_session, write_fd, SessionHolder};
use serde::Serialize;
use std::collections::HashMap;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

/// Public view of an active CLI OAuth login handshake.
#[derive(Clone, Debug, Serialize)]
pub struct OAuthLoginView {
    /// Target provider (`antigravity`, `claude`, `codex`).
    pub provider: String,
    /// Primary OAuth URL suitable for opening in an external browser.
    pub oauth_url: String,
    /// OAuth URL optimized for container-local Web-in-Web Chrome (`localhost` callback when available).
    pub web_in_web_url: String,
    /// True when pasting an authorization code or callback URL can complete sign-in.
    pub expects_code: bool,
}

struct ActiveLoginEntry {
    holder: SessionHolder,
    view: OAuthLoginView,
}

static OAUTH_SESSIONS: OnceLock<Mutex<HashMap<String, ActiveLoginEntry>>> = OnceLock::new();

fn sessions_map() -> &'static Mutex<HashMap<String, ActiveLoginEntry>> {
    OAUTH_SESSIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Returns the active [`OAuthLoginView`] for `provider`, if a login PTY is currently waiting.
pub async fn get_active_oauth_view(provider: &str) -> Option<OAuthLoginView> {
    sessions_map().lock().await.get(provider).map(|e| e.view.clone())
}

/// Terminates and removes any active CLI OAuth PTY for `provider`.
pub async fn cancel_oauth_login(provider: &str) {
    sessions_map().lock().await.remove(provider);
}

/// Strips ANSI CSI/OSC escape codes and carriage returns from raw PTY output bytes.
pub fn strip_ansi_codes(raw: &[u8]) -> String {
    let lossy = String::from_utf8_lossy(raw);
    static ANSI_RE: OnceLock<Option<regex::Regex>> = OnceLock::new();
    let re = ANSI_RE.get_or_init(|| regex::Regex::new(r"\x1b(?:\[[0-9;?]*[A-Za-z]|\][^\x07]*\x07)").ok());
    let cleaned = match re {
        Some(r) => r.replace_all(&lossy, "").into_owned(),
        None => lossy.into_owned(),
    };
    cleaned.replace('\r', "")
}

/// Reconstructs single-line or box-wrapped (`agy`) OAuth URLs from cleaned PTY text.
pub fn extract_oauth_url_from_pty(clean: &str) -> Option<String> {
    let lines: Vec<&str> = clean.lines().collect();
    for (idx, line) in lines.iter().enumerate() {
        if line.chars().filter(|&c| c == '─').count() >= 12 {
            let mut joined = String::new();
            for next in lines.iter().skip(idx + 1) {
                if next.chars().filter(|&c| c == '─').count() >= 12 {
                    break;
                }
                joined.push_str(next.trim());
            }
            if joined.starts_with("https://") || joined.starts_with("http://") {
                return Some(joined);
            }
        }
    }
    for token in clean.split_whitespace() {
        let trimmed = token.trim_matches(|c: char| matches!(c, '"' | '\'' | '<' | '>' | '(' | ')'));
        if trimmed.starts_with("https://") || trimmed.starts_with("http://") {
            return Some(trimmed.to_string());
        }
    }
    None
}

fn prepare_browser_hook(vault_dir: &Path, provider: &str) -> Result<(PathBuf, PathBuf), String> {
    std::fs::create_dir_all(vault_dir).map_err(|e| e.to_string())?;
    let url_file = vault_dir.join(format!(".cli_oauth_url_{provider}.txt"));
    let script_path = vault_dir.join(format!(".cli_browser_hook_{provider}.sh"));
    let _ = std::fs::remove_file(&url_file);
    let script = format!("#!/bin/sh\nprintf '%s\\n' \"$@\" >> \"{}\"\n", url_file.display());
    std::fs::write(&script_path, script).map_err(|e| e.to_string())?;
    let _ = std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o700));
    Ok((script_path, url_file))
}

fn build_login_args_and_env(
    provider: &str,
    home: &str,
    hook_script: &Path,
) -> (Vec<String>, HashMap<String, String>) {
    let args = match provider {
        "claude" => vec!["auth".to_string(), "login".to_string()],
        "codex" => vec!["login".to_string()],
        _ => Vec::new(),
    };
    let mut env: HashMap<String, String> = std::env::vars().collect();
    env.insert("HOME".to_string(), home.to_string());
    env.insert("BROWSER".to_string(), hook_script.display().to_string());
    env.insert("TERM".to_string(), "xterm-256color".to_string());
    env.remove("ANTHROPIC_API_KEY");
    env.remove("OPENAI_API_KEY");
    (args, env)
}

async fn poll_initial_oauth_urls(
    provider: &str,
    holder: &SessionHolder,
    url_file: &Path,
) -> Result<(String, String, bool), String> {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut sent_agy_enter = false;
    while Instant::now() < deadline {
        let raw_bytes = holder.output.lock().await.clone();
        let clean = strip_ansi_codes(&raw_bytes);
        if provider == "antigravity"
            && !sent_agy_enter
            && (clean.contains("Google OAuth") || clean.contains("Select login method"))
        {
            tokio::time::sleep(Duration::from_millis(180)).await;
            let _ = write_fd(holder.master_fd, "\r");
            sent_agy_enter = true;
        }
        let hook_url = std::fs::read_to_string(url_file)
            .ok()
            .and_then(|s| s.lines().find(|l| l.starts_with("http")).map(|l| l.trim().to_string()));
        let pty_url = extract_oauth_url_from_pty(&clean);

        if hook_url.is_some() || pty_url.is_some() {
            let web_in_web = hook_url.clone().or_else(|| pty_url.clone()).unwrap_or_default();
            let external = pty_url.or(hook_url).unwrap_or_else(|| web_in_web.clone());
            let expects_code = provider == "antigravity"
                || clean.contains("Paste code")
                || clean.contains("authorization code")
                || external.contains("127.0.0.1")
                || external.contains("localhost");
            return Ok((external, web_in_web, expects_code));
        }
        tokio::time::sleep(Duration::from_millis(160)).await;
    }
    let tail = strip_ansi_codes(&holder.output.lock().await);
    Err(format!("Timed out waiting for `{provider}` OAuth URL. Output: {}", tail.chars().take(240).collect::<String>()))
}

fn spawn_oauth_completion_watcher(ctx: Arc<WebAppCtx>, provider: String) {
    tokio::spawn(async move {
        let home = current_home_dir();
        let deadline = Instant::now() + Duration::from_secs(600);
        while Instant::now() < deadline {
            if !sessions_map().lock().await.contains_key(&provider) {
                return;
            }
            if inspect_provider(&home, &ctx.tuner_home, &provider).authenticated {
                sessions_map().lock().await.remove(&provider);
                ctx.events
                    .publish(WebEvent::ProviderSetupUpdated { provider })
                    .await;
                return;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        sessions_map().lock().await.remove(&provider);
    });
}

/// Spawns an interactive CLI OAuth login session and returns its [`OAuthLoginView`].
pub async fn start_provider_oauth(
    ctx: Arc<WebAppCtx>,
    provider: &str,
) -> Result<OAuthLoginView, String> {
    if !matches!(provider, "antigravity" | "claude" | "codex") {
        return Err(format!("Unsupported provider `{provider}`"));
    }
    cancel_oauth_login(provider).await;
    let home = current_home_dir();
    let Some(bin_path) = find_cli_binary(&home, provider) else {
        return Err(format!("`{provider}` CLI is not installed yet. Click Install first."));
    };
    if provider == "antigravity" {
        ensure_agy_headless_settings(&home, &ctx.working_dir);
    }
    let _ = std::fs::create_dir_all(&ctx.working_dir);
    let (hook_script, url_file) = prepare_browser_hook(&ctx.vault_dir, provider)?;
    let (args, env) = build_login_args_and_env(provider, &home, &hook_script);
    let holder = spawn_session(&ctx.working_dir, &bin_path.to_string_lossy(), &args, &env)?;

    let (oauth_url, web_in_web_url, expects_code) =
        poll_initial_oauth_urls(provider, &holder, &url_file).await?;
    let view = OAuthLoginView {
        provider: provider.to_string(),
        oauth_url,
        web_in_web_url,
        expects_code,
    };
    sessions_map().lock().await.insert(
        provider.to_string(),
        ActiveLoginEntry {
            holder,
            view: view.clone(),
        },
    );
    spawn_oauth_completion_watcher(ctx, provider.to_string());
    Ok(view)
}

fn extract_code_from_input(raw: &str) -> String {
    let trimmed = raw.trim();
    if (trimmed.starts_with("https://") || trimmed.starts_with("http://"))
        && let Ok(parsed) = url::Url::parse(trimmed)
        && let Some((_, val)) = parsed.query_pairs().find(|(k, _)| k == "code")
    {
        return val.into_owned();
    }
    trimmed.lines().next().unwrap_or("").trim().to_string()
}

/// Submits an authorization code or loopback callback URL to complete an active OAuth session.
pub async fn submit_provider_oauth_input(
    ctx: &WebAppCtx,
    provider: &str,
    raw_input: &str,
) -> Result<bool, String> {
    let trimmed = raw_input.trim();
    if trimmed.is_empty() {
        return Err("Authorization code or callback URL is empty".to_string());
    }
    if trimmed.starts_with("http://127.0.0.1:") || trimmed.starts_with("http://localhost:") {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(8))
            .build()
            .map_err(|e| e.to_string())?;
        let _ = client.get(trimmed).send().await.map_err(|e| format!("Callback relay failed: {e}"))?;
    } else {
        let code = extract_code_from_input(trimmed);
        let guard = sessions_map().lock().await;
        let Some(entry) = guard.get(provider) else {
            return Err(format!("No active `{provider}` login session. Start login again."));
        };
        write_fd(entry.holder.master_fd, &format!("{code}\r"))?;
    }

    let home = current_home_dir();
    let deadline = Instant::now() + Duration::from_secs(12);
    while Instant::now() < deadline {
        if inspect_provider(&home, &ctx.tuner_home, provider).authenticated {
            cancel_oauth_login(provider).await;
            ctx.events
                .publish(WebEvent::ProviderSetupUpdated {
                    provider: provider.to_string(),
                })
                .await;
            return Ok(true);
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    Ok(false)
}
