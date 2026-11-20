//! # Web-in-Web CLI OAuth Auto-Harvester & Completion Watcher (`provider_oauth_watch.rs`)
//!
//! ## Overview
//! Polls active Web-in-Web HITL Chrome sessions (`cli_antigravity`, `cli_claude`, `cli_codex`)
//! over loopback IPC to automatically harvest PKCE authorization codes (`[data-code-box]` or
//! `?code=`) into the waiting CLI PTY, auto-saving & closing the remote browser once authenticated.
//!
//! ## Search Tags
//! #oauth-auto-capture, #hitl-code-harvest, #cli-login-watcher

use super::api_hitl::{launch_hitl_session, StartHitlPayload};
use super::auth_http::{json_err, require_mutation_auth};
use super::events::WebEvent;
use super::provider_oauth::{cancel_oauth_login, feed_oauth_pty_line, has_active_oauth_session};
use super::provider_status::{
    current_home_dir, inspect_provider, read_dotenv_map, upsert_dotenv_secret,
};
use super::{resolve_ctx, WebAppCtx};
use axum::{
    http::HeaderMap,
    response::{IntoResponse, Response},
    Extension, Json,
};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
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

fn mask_secret(raw: &str) -> String {
    let t = raw.trim();
    let chars: Vec<char> = t.chars().collect();
    if chars.len() >= 10 {
        let head: String = chars[..4].iter().collect();
        let tail: String = chars[chars.len() - 4..].iter().collect();
        format!("{head}…{tail}")
    } else {
        "••••".to_string()
    }
}

/// Inspects `GEMINI_API_KEY` readiness and active Built-in Browser state for `/dashboard/api/providers/status`.
pub async fn inspect_gemini_api_status(tuner_home: &Path) -> serde_json::Value {
    let dotenv = read_dotenv_map(&tuner_home.join(".env"));
    let val = dotenv
        .get("GEMINI_API_KEY")
        .cloned()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| std::env::var("GEMINI_API_KEY").ok().filter(|s| !s.trim().is_empty()));
    let browser_active = crate::security::vault_auth::proxy::find_active_route_by_site("google")
        .await
        .is_some();
    serde_json::json!({
        "configured": val.is_some(),
        "masked_key": val.as_deref().map(mask_secret),
        "browser_active": browser_active
    })
}

fn seed_google_state_from_agy_if_missing(vault_dir: &Path) {
    let google_state = vault_dir.join("google_storage_state.json");
    let agy_state = vault_dir.join("cli_antigravity_storage_state.json");
    if !google_state.exists() && agy_state.is_file() {
        let _ = std::fs::create_dir_all(vault_dir);
        if std::fs::copy(&agy_state, &google_state).is_ok() {
            let _ = std::fs::set_permissions(&google_state, std::fs::Permissions::from_mode(0o600));
        }
    }
}

/// Polls an active `google` (`aistudio.google.com/apikey`) HITL browser for a newly generated `AIza...` key.
pub fn spawn_gemini_key_watcher(ctx: Arc<WebAppCtx>) {
    tokio::spawn(async move {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap_or_default();
        let deadline = Instant::now() + Duration::from_secs(600);
        while Instant::now() < deadline {
            let Some((tok, port)) =
                crate::security::vault_auth::proxy::find_active_route_by_site("google").await
            else {
                return;
            };
            if let Ok(resp) = client
                .get(format!("http://127.0.0.1:{port}/auth/{tok}/oauth_code"))
                .send()
                .await
                && let Ok(body) = resp.json::<serde_json::Value>().await
                && let Some(key) = body
                    .get("code")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| s.starts_with("AIza") && s.len() == 39)
                && upsert_dotenv_secret(&ctx.tuner_home.join(".env"), "GEMINI_API_KEY", key).is_ok()
            {
                let _ = client
                    .post(format!("http://127.0.0.1:{port}/auth/{tok}/finish"))
                    .send()
                    .await;
                ctx.events
                    .publish(WebEvent::ProviderSetupUpdated {
                        provider: "gemini".to_string(),
                    })
                    .await;
                return;
            }
            tokio::time::sleep(Duration::from_millis(450)).await;
        }
    });
}

/// Launches Google AI Studio (`aistudio.google.com/apikey`) inside the Built-in Browser and starts auto-harvest (`POST /dashboard/api/providers/gemini/browser`).
pub async fn handle_gemini_browser_start(
    ext: Option<Extension<Arc<WebAppCtx>>>,
    headers: HeaderMap,
) -> Response {
    let ctx = match resolve_ctx(ext) {
        Ok(c) => c,
        Err(r) => return *r,
    };
    if let Err(r) = require_mutation_auth(&ctx, &headers).await {
        return *r;
    }
    seed_google_state_from_agy_if_missing(&ctx.vault_dir);
    let target_url = std::env::var("TUNER_AISTUDIO_APIKEY_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "https://aistudio.google.com/apikey".to_string());
    let req = StartHitlPayload {
        target: "google".to_string(),
        url: Some(target_url),
        ttl_secs: 600,
    };
    match launch_hitl_session(&ctx.profile, &req).await {
        Ok(session) => {
            spawn_gemini_key_watcher(ctx);
            Json(serde_json::json!({ "ok": true, "hitl_session": session })).into_response()
        }
        Err((code, msg)) => json_err(code, &msg),
    }
}
