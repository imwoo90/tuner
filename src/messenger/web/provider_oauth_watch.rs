//! # Web-in-Web CLI OAuth Auto-Harvester & Completion Watcher (`provider_oauth_watch.rs`)
//!
//! ## Overview
//! Polls active Web-in-Web HITL Chrome sessions (`cli_antigravity`, `cli_claude`, `cli_codex`)
//! over loopback IPC to automatically harvest PKCE authorization codes (`[data-code-box]` or
//! `?code=`) into the waiting CLI PTY, auto-saving & closing the remote browser once authenticated.
//!
//! ## Search Tags
//! #oauth-auto-capture, #hitl-code-harvest, #cli-login-watcher

use super::events::WebEvent;
use super::provider_oauth::{cancel_oauth_login, feed_oauth_pty_line, has_active_oauth_session};
use super::provider_status::{current_home_dir, inspect_provider};
use super::WebAppCtx;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Saves cookies and closes any open `cli_<provider>` HITL remote browser session.
pub async fn auto_close_cli_hitl_browser(client: &reqwest::Client, provider: &str) {
    let site_key = format!("cli_{provider}");
    if let Some((tok, port)) =
        crate::security::vault_auth::proxy::find_active_route_by_site(&site_key).await
    {
        let _ = client
            .post(format!("http://127.0.0.1:{port}/auth/{tok}/finish"))
            .send()
            .await;
    }
}

async fn try_auto_harvest_hitl_code(
    client: &reqwest::Client,
    provider: &str,
    last_code: &mut Option<String>,
) {
    let site_key = format!("cli_{provider}");
    let Some((tok, port)) =
        crate::security::vault_auth::proxy::find_active_route_by_site(&site_key).await
    else {
        return;
    };
    let Ok(resp) = client
        .get(format!("http://127.0.0.1:{port}/auth/{tok}/oauth_code"))
        .send()
        .await
    else {
        return;
    };
    let Ok(body) = resp.json::<serde_json::Value>().await else {
        return;
    };
    let Some(code) = body
        .get("code")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return;
    };
    if last_code.as_deref() == Some(code) {
        return;
    }
    if feed_oauth_pty_line(provider, code).await.is_ok() {
        *last_code = Some(code.to_string());
    }
}

/// Spawns the background watcher that auto-harvests Web-in-Web OAuth codes and emits `ProviderSetupUpdated`.
pub fn spawn_oauth_completion_watcher(ctx: Arc<WebAppCtx>, provider: String) {
    tokio::spawn(async move {
        let home = current_home_dir();
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap_or_default();
        let mut last_code: Option<String> = None;
        let deadline = Instant::now() + Duration::from_secs(600);
        while Instant::now() < deadline {
            if !has_active_oauth_session(&provider).await {
                return;
            }
            if inspect_provider(&home, &ctx.tuner_home, &provider).authenticated {
                auto_close_cli_hitl_browser(&client, &provider).await;
                cancel_oauth_login(&provider).await;
                ctx.events
                    .publish(WebEvent::ProviderSetupUpdated { provider })
                    .await;
                return;
            }
            try_auto_harvest_hitl_code(&client, &provider, &mut last_code).await;
            tokio::time::sleep(Duration::from_millis(450)).await;
        }
        cancel_oauth_login(&provider).await;
    });
}
