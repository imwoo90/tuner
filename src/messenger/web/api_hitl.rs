//! # In-Dashboard HITL Remote Browser Control API (`api_hitl.rs`)
//!
//! ## Overview
//! Exposes active `/auth/:token` Web-in-Web Human-in-the-Loop browser sessions to
//! authenticated Dashboard clients and allows launching on-demand remote Chrome
//! authentication sessions (`tuner --vault auth`) directly from the Dashboard UI.
//!
//! ## Search Tags
//! #web-hitl, #remote-browser, #vault-auth-ui

use super::auth_http::{json_err, require_auth, require_mutation_auth};
use super::{resolve_ctx, WebAppCtx};
use crate::security::vault_auth::cdp_state::validate_http_url;
use crate::security::vault_auth::options::canonical_storage_slug;
use crate::security::vault_auth::proxy::list_active_auth_sessions;
use axum::{
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Extension, Json,
};
use serde::Deserialize;
use std::collections::HashSet;
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Lists active HITL remote browser sessions (`GET /dashboard/api/hitl`).
pub async fn handle_hitl_status(
    ext: Option<Extension<Arc<WebAppCtx>>>,
    headers: HeaderMap,
) -> Response {
    let ctx = match resolve_ctx(ext) {
        Ok(c) => c,
        Err(r) => return *r,
    };
    if let Err(r) = require_auth(&ctx, &headers).await {
        return *r;
    }
    let sessions = list_active_auth_sessions().await;
    Json(serde_json::json!({
        "active": !sessions.is_empty(),
        "sessions": sessions
    }))
    .into_response()
}

/// Request payload for `POST /dashboard/api/hitl/start`.
#[derive(Deserialize)]
pub struct StartHitlPayload {
    /// Vault `site_key` slug (e.g. `github`) or `https://...` URL.
    pub target: String,
    /// Optional explicit start URL when `target` is a new `site_key` not yet in Vault.
    #[serde(default)]
    pub url: Option<String>,
    /// Optional session TTL in seconds (`60..=1800`, default `600`).
    #[serde(default = "default_hitl_ttl")]
    pub ttl_secs: u64,
}

fn default_hitl_ttl() -> u64 {
    600
}

fn build_vault_auth_args(profile: &str, payload: &StartHitlPayload) -> Result<Vec<String>, String> {
    let raw = payload.target.trim();
    if raw.is_empty() {
        return Err("Target site_key or URL is required".to_string());
    }
    let ttl = payload.ttl_secs.clamp(60, 1800).to_string();
    let mut args = vec!["--profile".to_string(), profile.to_string(), "--vault".to_string(), "auth".to_string()];
    if raw.starts_with("http://") || raw.starts_with("https://") {
        let valid_url = validate_http_url(raw)?;
        args.push(valid_url);
    } else {
        let slug = canonical_storage_slug(raw);
        if slug.is_empty() || slug == "site" && raw != "site" {
            return Err("Invalid site_key format".to_string());
        }
        args.push(slug);
        if let Some(u) = payload.url.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            let valid_url = validate_http_url(u)?;
            args.push("--url".to_string());
            args.push(valid_url);
        }
    }
    args.push("--ttl".to_string());
    args.push(ttl);
    Ok(args)
}

async fn wait_for_hitl_route(
    mut child: tokio::process::Child,
    known: HashSet<String>,
) -> Response {
    let deadline = Instant::now() + Duration::from_secs(12);
    while Instant::now() < deadline {
        if let Ok(Some(status)) = child.try_wait() {
            return json_err(
                StatusCode::BAD_REQUEST,
                &format!("Remote browser exited early ({status}). Ensure URL is configured in Vault or provide https:// URL."),
            );
        }
        let now_list = list_active_auth_sessions().await;
        if let Some(found) = now_list.iter().find(|s| !known.contains(&s.token)) {
            let info = found.clone();
            tokio::spawn(async move {
                let _ = child.wait().await;
            });
            return (StatusCode::CREATED, Json(serde_json::json!({ "ok": true, "session": info }))).into_response();
        }
        tokio::time::sleep(Duration::from_millis(180)).await;
    }
    let _ = child.kill().await;
    json_err(StatusCode::GATEWAY_TIMEOUT, "Timed out waiting for remote Chrome display to initialize")
}

/// Spawns a background `tuner --vault auth` session and waits for its `/auth/:token` route (`POST /dashboard/api/hitl/start`).
pub async fn handle_hitl_start(
    ext: Option<Extension<Arc<WebAppCtx>>>,
    headers: HeaderMap,
    Json(payload): Json<StartHitlPayload>,
) -> Response {
    let ctx = match resolve_ctx(ext) {
        Ok(c) => c,
        Err(r) => return *r,
    };
    if let Err(r) = require_mutation_auth(&ctx, &headers).await {
        return *r;
    }
    let before = list_active_auth_sessions().await;
    if before.len() >= 2 {
        return json_err(StatusCode::CONFLICT, "Maximum concurrent remote browser sessions (2) already active");
    }
    let cli_args = match build_vault_auth_args(&ctx.profile, &payload) {
        Ok(a) => a,
        Err(e) => return json_err(StatusCode::BAD_REQUEST, &e),
    };
    let Ok(exe) = std::env::current_exe() else {
        return json_err(StatusCode::INTERNAL_SERVER_ERROR, "Failed to resolve tuner executable");
    };
    let known: HashSet<String> = before.into_iter().map(|s| s.token).collect();
    let child = match tokio::process::Command::new(exe)
        .args(&cli_args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => return json_err(StatusCode::INTERNAL_SERVER_ERROR, &format!("Failed to launch browser auth: {e}")),
    };
    wait_for_hitl_route(child, known).await
}
