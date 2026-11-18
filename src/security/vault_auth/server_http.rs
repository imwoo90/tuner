//! # Loopback HITL HTTP Handlers & OAuth Probe (`server_http.rs`)
//!
//! ## Overview
//! Serves the token-gated `vault_auth_viewer.html` document, upstream SSRF ping magic,
//! loopback OAuth callback code extraction (`/oauth_code`), and programmatic completion (`/finish`).
//!
//! ## Search Tags
//! #hitl-http-routes, #oauth-probe, #viewer-bootstrap

use super::cdp_state::export_storage_state;
use super::proxy::HITL_PING_MAGIC;
use super::server::AuthServerState;
use crate::security::vault::current_unix_secs;
use axum::{
    extract::{Path as AxumPath, State},
    http::{header, StatusCode},
    response::{Html, IntoResponse, Response},
    Json,
};
use serde_json::json;
use std::sync::Arc;

pub(crate) async fn handle_viewer_ping(
    AxumPath(token): AxumPath<String>,
    State(state): State<Arc<AuthServerState>>,
) -> Response {
    if !state.is_valid_token(&token) || state.is_saved_done() {
        return StatusCode::FORBIDDEN.into_response();
    }
    let mut resp = (StatusCode::OK, HITL_PING_MAGIC).into_response();
    if let Ok(hv) = header::HeaderValue::from_str(&state.site_key) {
        resp.headers_mut()
            .insert(header::HeaderName::from_static("x-tuner-site-key"), hv);
    }
    resp
}

pub(crate) async fn handle_oauth_code(
    AxumPath(token): AxumPath<String>,
    State(state): State<Arc<AuthServerState>>,
) -> Response {
    if !state.is_valid_token(&token) || state.is_saved_done() {
        return StatusCode::FORBIDDEN.into_response();
    }
    let code = state.cdp.probe_oauth_code().await;
    Json(json!({ "code": code })).into_response()
}

pub(crate) async fn handle_auto_finish(
    AxumPath(token): AxumPath<String>,
    State(state): State<Arc<AuthServerState>>,
) -> Response {
    if !state.is_valid_token(&token) || state.is_saved_done() {
        return StatusCode::FORBIDDEN.into_response();
    }
    if let Ok(summary) =
        export_storage_state(state.cdp_port, &state.browser_ws_url, &state.state_path).await
    {
        *state.done_summary.lock().await = Some(summary);
    }
    state.mark_auto_finished();
    Json(json!({ "ok": true })).into_response()
}

pub(crate) async fn handle_viewer_html(
    AxumPath(token): AxumPath<String>,
    State(state): State<Arc<AuthServerState>>,
) -> Response {
    if !state.is_valid_token(&token) || state.is_saved_done() {
        return (
            StatusCode::FORBIDDEN,
            "Remote auth session expired or already completed.",
        )
            .into_response();
    }
    let rem_ttl = state.expires_at_secs.saturating_sub(current_unix_secs());
    let boot = json!({
        "site_key": state.site_key,
        "ttl_secs": rem_ttl,
        "has_username": state.cdp.has_username(),
        "has_password": state.cdp.has_password(),
    });
    let safe_json = boot
        .to_string()
        .replace('<', "\\u003c")
        .replace('>', "\\u003e");
    let html = include_str!("vault_auth_viewer.html").replace("__AUTH_BOOTSTRAP__", &safe_json);
    let mut resp = Html(html).into_response();
    if let Ok(hv) = header::HeaderValue::from_str("no-store, no-cache, must-revalidate") {
        resp.headers_mut().insert(header::CACHE_CONTROL, hv);
    }
    resp
}
