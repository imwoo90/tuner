//! # Dashboard CLI & Telegram `/dashboard` Entrypoint (`cli_dash.rs`)
//!
//! ## Overview
//! Handles `tuner --dashboard [--profile <name>] [--set-pin <pin>]` CLI magic-link
//! issuance and PIN setup, live tunnel metadata sharing, and the Telegram `/dashboard`
//! owner command with inline launch button.
//!
//! ## Search Tags
//! #dashboard-cli, #magic-link-cli, #telegram-dashboard, #tailscale-url

use super::auth::WebAuthStore;
use crate::config::CliConfig;
use crate::messenger::telegram::commands_remote_hub::{check_tailscale_status, ServiceStatus};
use crate::security::vault_auth::proxy::{load_tunnel_endpoint, set_mode_0600};
use crate::workspace::paths::{extract_profile_from_working_dir, resolve_paths};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, ParseMode};

const TUNNEL_STATE_FILE: &str = "dashboard_tunnel.json";

#[derive(Serialize, Deserialize)]
struct TunnelStateFile {
    pid: u32,
    url: String,
}

/// Publishes or clears the global Web Dashboard Quick Tunnel URL if `WebAppCtx` is initialized.
pub fn publish_global_tunnel_url(url: Option<&str>) {
    if let Some(ctx) = super::global_web_ctx() {
        publish_dashboard_tunnel_url(&ctx.vault_dir, url);
    }
}

/// Persists or clears the active Quick Tunnel URL (`0600`) for CLI `--dashboard` discovery.
pub fn publish_dashboard_tunnel_url(vault_dir: &Path, url: Option<&str>) {
    let dst = vault_dir.join(TUNNEL_STATE_FILE);
    match url {
        Some(u) if !u.trim().is_empty() => {
            let _ = std::fs::create_dir_all(vault_dir);
            let tmp = vault_dir.join(".dashboard_tunnel.json.tmp");
            let payload = TunnelStateFile {
                pid: std::process::id(),
                url: u.trim().to_string(),
            };
            if let Ok(raw) = serde_json::to_string(&payload)
                && std::fs::write(&tmp, raw).is_ok()
            {
                set_mode_0600(&tmp);
                let _ = std::fs::rename(&tmp, &dst);
                set_mode_0600(&dst);
            }
        }
        _ => {
            let _ = std::fs::remove_file(&dst);
        }
    }
}

/// Loads an active Quick Tunnel URL from `dashboard_tunnel.json` if its owner PID is alive.
pub fn load_dashboard_tunnel_url(vault_dir: &Path) -> Option<String> {
    let raw = std::fs::read_to_string(vault_dir.join(TUNNEL_STATE_FILE)).ok()?;
    let st: TunnelStateFile = serde_json::from_str(&raw).ok()?;
    if st.pid == 0 || st.url.is_empty() {
        return None;
    }
    #[cfg(target_os = "linux")]
    if !Path::new(&format!("/proc/{}", st.pid)).exists() {
        return None;
    }
    Some(st.url)
}

/// Parses `--dashboard [--profile <name>] [--set-pin <pin>]` CLI arguments.
pub fn parse_dashboard_cli_args(args: &[String]) -> Result<(String, Option<String>), String> {
    let mut profile = "default".to_string();
    let mut set_pin = None;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--dashboard" => i += 1,
            "--profile" => {
                let val = args
                    .get(i + 1)
                    .filter(|s| !s.trim().is_empty() && !s.starts_with('-'))
                    .ok_or_else(|| "Missing value for --profile".to_string())?;
                profile = val.clone();
                i += 2;
            }
            "--set-pin" => {
                let val = args
                    .get(i + 1)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| "Missing value for --set-pin".to_string())?;
                set_pin = Some(val.clone());
                i += 2;
            }
            other => {
                return Err(format!(
                    "Unknown option '{other}' for --dashboard. Usage: tuner --dashboard [--profile <name>] [--set-pin <pin>]"
                ));
            }
        }
    }
    Ok((profile, set_pin))
}

/// Executes `tuner --dashboard [--profile <name>] [--set-pin <pin>]`.
pub async fn run_dashboard_cli(args: &[String]) -> Result<(), String> {
    let (profile, set_pin) = parse_dashboard_cli_args(args)?;
    let paths = resolve_paths(None, None, None, Some(profile.clone()));
    let _ = crate::setup::load_env_file(&paths.env_file());
    let vault_dir = paths.vault_dir();
    let store = WebAuthStore::new(vault_dir.clone());
    if let Some(ref pin) = set_pin {
        store.set_pin(pin).await?;
        println!("🔐 [tuner] Owner dashboard PIN updated for profile '{profile}'.");
    }
    let token = store.issue_magic_token().await?;
    let port = load_tunnel_endpoint(&vault_dir).map(|e| e.port).unwrap_or(8743);
    let local_url = format!("http://127.0.0.1:{port}/dashboard/magic/{token}");
    let ts_url = match check_tailscale_status() {
        (ServiceStatus::Active, Some(ip)) => Some(format!("http://{ip}:{port}/dashboard/magic/{token}")),
        _ => None,
    };
    let tunnel_url = load_dashboard_tunnel_url(&vault_dir)
        .map(|base| format!("{}/dashboard/magic/{token}", base.trim_end_matches('/')));
    println!("🖥️ [tuner] Web Dashboard Magic Link (single-use, 10m TTL, profile: {profile}):");
    println!("  • Local     : {local_url}");
    if let Some(ref u) = ts_url {
        println!("  • Tailscale : {u}");
    }
    if let Some(ref u) = tunnel_url {
        println!("  • Tunnel    : {u}");
    }
    Ok(())
}

fn resolve_dashboard_target(config: &CliConfig) -> (String, PathBuf, u16) {
    if let Some(ctx) = super::global_web_ctx() {
        let port = load_tunnel_endpoint(&ctx.vault_dir).map(|e| e.port).unwrap_or(8743);
        return (ctx.profile.clone(), ctx.vault_dir.clone(), port);
    }
    if let Some((prof, vdir)) = extract_profile_from_working_dir(&config.working_dir) {
        let port = load_tunnel_endpoint(&vdir).map(|e| e.port).unwrap_or(8743);
        return (prof, vdir, port);
    }
    let paths = resolve_paths(None, None, None, Some("default".to_string()));
    let vdir = paths.vault_dir();
    let port = load_tunnel_endpoint(&vdir).map(|e| e.port).unwrap_or(8743);
    ("default".to_string(), vdir, port)
}

fn format_dashboard_html_and_kb(
    profile: &str,
    local_url: &str,
    ts_url: Option<&str>,
    tunnel_url: Option<&str>,
) -> (String, Vec<Vec<InlineKeyboardButton>>) {
    let mut lines = vec![
        format!("🖥️ <b>[Tuner Web Dashboard]</b> (프로필: <code>{}</code>)", html_escape::encode_text(profile)),
        String::new(),
        "• 아래 링크는 <b>1회용(10분 유효)</b> 자동 로그인 Magic Link입니다.".to_string(),
        format!("• <b>Local</b>: <code>{}</code>", html_escape::encode_text(local_url)),
    ];
    if let Some(u) = ts_url {
        lines.push(format!("• <b>Tailscale</b>: <code>{}</code>", html_escape::encode_text(u)));
    }
    if let Some(u) = tunnel_url {
        lines.push(format!("• <b>Tunnel</b>: <code>{}</code>", html_escape::encode_text(u)));
    }
    let mut rows = Vec::new();
    if let Some(u) = tunnel_url
        && u.starts_with("https://")
        && let Ok(parsed) = url::Url::parse(u)
    {
        rows.push(vec![InlineKeyboardButton::url("🖥️ Tuner Web Dashboard 열기", parsed)]);
    }
    (lines.join("\n"), rows)
}

async fn send_dashboard_card(
    bot: &Bot,
    msg: &Message,
    text: String,
    rows: Vec<Vec<InlineKeyboardButton>>,
) {
    let no_preview = teloxide::types::LinkPreviewOptions {
        is_disabled: true,
        url: None,
        prefer_small_media: false,
        prefer_large_media: false,
        show_above_text: false,
    };
    let mut req = bot
        .send_message(msg.chat.id, text.clone())
        .parse_mode(ParseMode::Html)
        .link_preview_options(no_preview.clone());
    if let Some(tid) = msg.thread_id {
        req = req.message_thread_id(tid);
    }
    if !rows.is_empty() {
        if req.reply_markup(InlineKeyboardMarkup::new(rows)).await.is_ok() {
            return;
        }
        let mut fb = bot
            .send_message(msg.chat.id, text)
            .parse_mode(ParseMode::Html)
            .link_preview_options(no_preview);
        if let Some(tid) = msg.thread_id {
            fb = fb.message_thread_id(tid);
        }
        let _ = fb.await;
    } else {
        let _ = req.await;
    }
}

/// Issues a fresh single-use Magic Link and sends the Dashboard card to Telegram.
pub async fn handle_telegram_dashboard_command(
    bot: &Bot,
    msg: &Message,
    config: &CliConfig,
) -> Result<(), teloxide::RequestError> {
    let (profile, vault_dir, port) = resolve_dashboard_target(config);
    let store = WebAuthStore::new(vault_dir.clone());
    let Ok(token) = store.issue_magic_token().await else {
        let _ = crate::messenger::telegram::commands::send_reply(bot, msg, "❌ Failed to issue dashboard magic link.").await;
        return Ok(());
    };
    let tunnel_base = crate::messenger::telegram::review::global_review_manager()
        .current_tunnel_url()
        .await
        .or_else(|| load_dashboard_tunnel_url(&vault_dir));
    let local_url = format!("http://127.0.0.1:{port}/dashboard/magic/{token}");
    let ts_url = match check_tailscale_status() {
        (ServiceStatus::Active, Some(ip)) => Some(format!("http://{ip}:{port}/dashboard/magic/{token}")),
        _ => None,
    };
    let tunnel_url = tunnel_base.map(|b| format!("{}/dashboard/magic/{token}", b.trim_end_matches('/')));
    let (text, rows) = format_dashboard_html_and_kb(&profile, &local_url, ts_url.as_deref(), tunnel_url.as_deref());
    send_dashboard_card(bot, msg, text, rows).await;
    Ok(())
}
