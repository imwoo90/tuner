//! # CLI Provider Setup & Onboarding HTTP Endpoints (`api_providers.rs`)
//!
//! ## Overview
//! Exposes `/dashboard/api/providers/*` endpoints for zero-terminal Docker / first-run
//! onboarding: inspecting `antigravity` / `claude` / `codex` installation & auth status,
//! triggering user-space CLI installs, launching Web-in-Web OAuth sign-in, submitting
//! authorization codes / callback URLs, and saving API keys.
//!
//! ## Search Tags
//! #api-providers, #first-run-onboarding, #web-in-web-login, #cli-install-api

use super::api_hitl::{launch_hitl_session, StartHitlPayload};
use super::auth_http::{json_err, require_auth, require_mutation_auth};
use super::events::WebEvent;
use super::provider_install::{get_install_job, start_provider_install};
use super::provider_oauth::{
    cancel_oauth_login, get_active_oauth_view, start_provider_oauth, submit_provider_oauth_input,
};
use super::provider_oauth_watch::{handle_gemini_browser_start, inspect_gemini_api_status};
use super::provider_status::{
    current_home_dir, find_cli_binary, inspect_all_providers, upsert_dotenv_secret,
};
use super::{resolve_ctx, WebAppCtx};
use axum::{
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Extension, Json, Router,
};
use serde::Deserialize;
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;

/// Request payload targeting a single provider (`POST /dashboard/api/providers/install` or `/login/cancel`).
#[derive(Deserialize)]
pub struct ProviderTargetPayload {
    /// Target provider (`antigravity`, `claude`, `codex`).
    pub provider: String,
}

/// Request payload for `POST /dashboard/api/providers/login/start`.
#[derive(Deserialize)]
pub struct StartLoginPayload {
    /// Target provider (`antigravity`, `claude`, `codex`).
    pub provider: String,
    /// When `true`, automatically opens the OAuth URL inside the embedded Web-in-Web Chrome.
    #[serde(default)]
    pub open_web_in_web: bool,
}

/// Request payload for `POST /dashboard/api/providers/login/submit`.
#[derive(Deserialize)]
pub struct SubmitLoginPayload {
    /// Target provider (`antigravity`, `claude`, `codex`).
    pub provider: String,
    /// Pasted authorization code or callback URL.
    pub code: String,
}

/// Request payload for `POST /dashboard/api/providers/apikey`.
#[derive(Deserialize)]
pub struct ApiKeyPayload {
    /// Target provider (`claude` or `codex`).
    pub provider: String,
    /// API key secret (`sk-ant-...` or `sk-...`).
    pub api_key: String,
}

/// Returns installation, authentication, installer log, and active login state (`GET /dashboard/api/providers/status`).
pub async fn handle_providers_status(
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
    let (any_ready, base_list) = inspect_all_providers(&ctx.tuner_home);
    let mut enriched = Vec::with_capacity(base_list.len());
    for item in base_list {
        let install_job = get_install_job(item.id).await;
        let oauth_login = get_active_oauth_view(item.id).await;
        enriched.push(serde_json::json!({
            "id": item.id,
            "name": item.name,
            "installed": item.installed,
            "authenticated": item.authenticated,
            "version": item.version,
            "binary_path": item.binary_path,
            "auth_detail": item.auth_detail,
            "install_job": install_job,
            "oauth_login": oauth_login
        }));
    }
    let gemini_api = inspect_gemini_api_status(&ctx.tuner_home).await;
    Json(serde_json::json!({
        "any_ready": any_ready,
        "providers": enriched,
        "gemini_api": gemini_api
    }))
    .into_response()
}

/// Starts a background CLI installation job (`POST /dashboard/api/providers/install`).
pub async fn handle_provider_install(
    ext: Option<Extension<Arc<WebAppCtx>>>,
    headers: HeaderMap,
    Json(payload): Json<ProviderTargetPayload>,
) -> Response {
    let ctx = match resolve_ctx(ext) {
        Ok(c) => c,
        Err(r) => return *r,
    };
    if let Err(r) = require_mutation_auth(&ctx, &headers).await {
        return *r;
    }
    let prov = payload.provider.trim().to_ascii_lowercase();
    match start_provider_install(ctx, &prov).await {
        Ok(()) => (StatusCode::ACCEPTED, Json(serde_json::json!({ "ok": true, "provider": prov }))).into_response(),
        Err(e) => json_err(StatusCode::BAD_REQUEST, &e),
    }
}

/// Starts an interactive CLI OAuth login handshake (`POST /dashboard/api/providers/login/start`).
pub async fn handle_provider_login_start(
    ext: Option<Extension<Arc<WebAppCtx>>>,
    headers: HeaderMap,
    Json(payload): Json<StartLoginPayload>,
) -> Response {
    let ctx = match resolve_ctx(ext) {
        Ok(c) => c,
        Err(r) => return *r,
    };
    if let Err(r) = require_mutation_auth(&ctx, &headers).await {
        return *r;
    }
    let prov = payload.provider.trim().to_ascii_lowercase();
    let view = match start_provider_oauth(ctx.clone(), &prov).await {
        Ok(v) => v,
        Err(e) => return json_err(StatusCode::BAD_REQUEST, &e),
    };
    let hitl_session = if payload.open_web_in_web {
        let req = StartHitlPayload {
            target: format!("cli_{prov}"),
            url: Some(view.web_in_web_url.clone()),
            ttl_secs: 600,
        };
        launch_hitl_session(&ctx.profile, &req).await.ok()
    } else {
        None
    };
    Json(serde_json::json!({
        "ok": true,
        "login": view,
        "hitl_session": hitl_session
    }))
    .into_response()
}

/// Submits an authorization code or callback URL (`POST /dashboard/api/providers/login/submit`).
pub async fn handle_provider_login_submit(
    ext: Option<Extension<Arc<WebAppCtx>>>,
    headers: HeaderMap,
    Json(payload): Json<SubmitLoginPayload>,
) -> Response {
    let ctx = match resolve_ctx(ext) {
        Ok(c) => c,
        Err(r) => return *r,
    };
    if let Err(r) = require_mutation_auth(&ctx, &headers).await {
        return *r;
    }
    let prov = payload.provider.trim().to_ascii_lowercase();
    match submit_provider_oauth_input(&ctx, &prov, &payload.code).await {
        Ok(authenticated) => Json(serde_json::json!({
            "ok": true,
            "provider": prov,
            "authenticated": authenticated
        }))
        .into_response(),
        Err(e) => json_err(StatusCode::BAD_REQUEST, &e),
    }
}

/// Cancels an active CLI OAuth login session (`POST /dashboard/api/providers/login/cancel`).
pub async fn handle_provider_login_cancel(
    ext: Option<Extension<Arc<WebAppCtx>>>,
    headers: HeaderMap,
    Json(payload): Json<ProviderTargetPayload>,
) -> Response {
    let ctx = match resolve_ctx(ext) {
        Ok(c) => c,
        Err(r) => return *r,
    };
    if let Err(r) = require_mutation_auth(&ctx, &headers).await {
        return *r;
    }
    let prov = payload.provider.trim().to_ascii_lowercase();
    cancel_oauth_login(&prov).await;
    Json(serde_json::json!({ "ok": true })).into_response()
}

/// Configures `ANTHROPIC_API_KEY` or `OPENAI_API_KEY` in `$TUNER_HOME/.env` (`POST /dashboard/api/providers/apikey`).
pub async fn handle_provider_apikey(
    ext: Option<Extension<Arc<WebAppCtx>>>,
    headers: HeaderMap,
    Json(payload): Json<ApiKeyPayload>,
) -> Response {
    let ctx = match resolve_ctx(ext) {
        Ok(c) => c,
        Err(r) => return *r,
    };
    if let Err(r) = require_mutation_auth(&ctx, &headers).await {
        return *r;
    }
    let prov = payload.provider.trim().to_ascii_lowercase();
    let key_val = payload.api_key.trim();
    if key_val.is_empty() {
        return json_err(StatusCode::BAD_REQUEST, "API key cannot be empty");
    }
    let env_name = match prov.as_str() {
        "claude" => "ANTHROPIC_API_KEY",
        "codex" => "OPENAI_API_KEY",
        "gemini" => "GEMINI_API_KEY",
        _ => return json_err(StatusCode::BAD_REQUEST, "API key setup is supported for `claude`, `codex`, and `gemini`"),
    };
    if let Err(e) = upsert_dotenv_secret(&ctx.tuner_home.join(".env"), env_name, key_val) {
        return json_err(StatusCode::INTERNAL_SERVER_ERROR, &e);
    }
    if prov == "codex"
        && let Some(codex_bin) = find_cli_binary(&current_home_dir(), "codex")
        && let Ok(mut child) = tokio::process::Command::new(codex_bin)
            .args(["login", "--with-api-key"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
    {
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(format!("{key_val}\n").as_bytes()).await;
        }
        let _ = child.wait().await;
    }
    ctx.events
        .publish(WebEvent::ProviderSetupUpdated { provider: prov.clone() })
        .await;
    Json(serde_json::json!({ "ok": true, "provider": prov })).into_response()
}

/// Builds the `/dashboard/api/providers/*` sub-router.
pub fn provider_routes() -> Router {
    Router::new()
        .route("/dashboard/api/providers/status", get(handle_providers_status))
        .route("/dashboard/api/providers/install", post(handle_provider_install))
        .route("/dashboard/api/providers/login/start", post(handle_provider_login_start))
        .route("/dashboard/api/providers/login/submit", post(handle_provider_login_submit))
        .route("/dashboard/api/providers/login/cancel", post(handle_provider_login_cancel))
        .route("/dashboard/api/providers/apikey", post(handle_provider_apikey))
        .route("/dashboard/api/providers/gemini/browser", post(handle_gemini_browser_start))
}
