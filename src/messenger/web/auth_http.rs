//! # HTTP Authentication & Session Cookie Handlers (`auth_http.rs`)
//!
//! ## Overview
//! Implements cookie parsing, CSRF header validation, Magic Link token exchange,
//! PBKDF2 PIN login/update, session status queries, and logout/revocation endpoints.
//!
//! ## Search Tags
//! #web-auth-http, #csrf-guard, #magic-exchange, #pin-login

use super::auth::AuthSession;
use super::{resolve_ctx, WebAppCtx};
use crate::security::vault_auth::proxy::constant_time_eq;
use axum::{
    extract::{ConnectInfo, Path},
    http::{header, HeaderMap, StatusCode},
    response::{Html, IntoResponse, Response},
    Extension, Json,
};
use serde::Deserialize;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;

const COOKIE_NAME: &str = "tuner_dash_sid";
const CSRF_HEADER: &str = "x-tuner-csrf";

/// Extracts the `tuner_dash_sid` cookie value from HTTP request headers.
pub fn extract_session_cookie(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get(header::COOKIE)?.to_str().ok()?;
    for part in raw.split(';') {
        if let Some((k, v)) = part.trim().split_once('=')
            && k.trim() == COOKIE_NAME
            && !v.trim().is_empty()
        {
            return Some(v.trim().to_string());
        }
    }
    None
}

fn is_https_request(headers: &HeaderMap) -> bool {
    headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|p| p.trim().eq_ignore_ascii_case("https"))
}

fn build_session_cookie(sid: &str, max_age_secs: i64, secure: bool) -> String {
    let mut s = format!("{COOKIE_NAME}={sid}; HttpOnly; SameSite=Lax; Path=/; Max-Age={max_age_secs}");
    if secure {
        s.push_str("; Secure");
    }
    s
}

fn extract_client_ip(headers: &HeaderMap, peer: Option<SocketAddr>) -> IpAddr {
    let peer_ip = peer.map(|s| s.ip());
    if peer_ip.is_some_and(|ip| ip.is_loopback())
        && headers.contains_key("cf-ray")
        && let Some(ip) = headers
            .get("cf-connecting-ip")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.trim().parse::<IpAddr>().ok())
    {
        return ip;
    }
    peer_ip.unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST))
}

fn is_preview_bot_or_prefetch(headers: &HeaderMap) -> bool {
    for hdr in ["sec-purpose", "purpose", "x-moz"] {
        if headers
            .get(hdr)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.to_ascii_lowercase().contains("prefetch"))
        {
            return true;
        }
    }
    if let Some(ua) = headers.get(header::USER_AGENT).and_then(|v| v.to_str().ok()) {
        let low = ua.to_ascii_lowercase();
        if [
            "telegrambot",
            "slackbot",
            "discordbot",
            "whatsapp",
            "twitterbot",
            "facebookexternalhit",
            "kakaotalk-scrap",
        ]
        .iter()
        .any(|b| low.contains(b))
        {
            return true;
        }
    }
    false
}

/// Authenticates a read-only (`GET`) API request via `tuner_dash_sid` cookie.
pub async fn require_auth(
    ctx: &Arc<WebAppCtx>,
    headers: &HeaderMap,
) -> Result<AuthSession, Box<Response>> {
    let Some(sid) = extract_session_cookie(headers) else {
        return Err(Box::new(json_err(StatusCode::UNAUTHORIZED, "Authentication required")));
    };
    ctx.auth
        .verify_session(&sid)
        .await
        .ok_or_else(|| Box::new(json_err(StatusCode::UNAUTHORIZED, "Invalid or expired session")))
}

/// Authenticates a mutating API request via both `tuner_dash_sid` cookie and `x-tuner-csrf`.
pub async fn require_mutation_auth(
    ctx: &Arc<WebAppCtx>,
    headers: &HeaderMap,
) -> Result<AuthSession, Box<Response>> {
    let sess = require_auth(ctx, headers).await?;
    let Some(csrf) = headers.get(CSRF_HEADER).and_then(|v| v.to_str().ok()) else {
        return Err(Box::new(json_err(StatusCode::FORBIDDEN, "Missing CSRF token")));
    };
    if !constant_time_eq(csrf.as_bytes(), sess.csrf_token.as_bytes()) {
        return Err(Box::new(json_err(StatusCode::FORBIDDEN, "Invalid CSRF token")));
    }
    Ok(sess)
}

/// Helper building a JSON error response with the given status code.
pub fn json_err(status: StatusCode, msg: &str) -> Response {
    (status, Json(serde_json::json!({ "error": msg }))).into_response()
}

/// Exchanges a single-use Magic Link token (`GET /dashboard/magic/:token`) for a session cookie.
pub async fn handle_magic_exchange(
    ext: Option<Extension<Arc<WebAppCtx>>>,
    Path(token): Path<String>,
    headers: HeaderMap,
) -> Response {
    if is_preview_bot_or_prefetch(&headers) {
        return StatusCode::NO_CONTENT.into_response();
    }
    let ctx = match resolve_ctx(ext) {
        Ok(c) => c,
        Err(r) => return *r,
    };
    match ctx.auth.consume_magic_token(&token).await {
        Ok(Some(sess)) => {
            let cookie = build_session_cookie(&sess.sid, 30 * 86_400, is_https_request(&headers));
            let mut res = (StatusCode::SEE_OTHER, [(header::LOCATION, "/dashboard")]).into_response();
            if let Ok(hv) = header::HeaderValue::from_str(&cookie) {
                res.headers_mut().insert(header::SET_COOKIE, hv);
            }
            res
        }
        _ => (
            StatusCode::UNAUTHORIZED,
            Html(
                "<!doctype html><html><body style='background:#FAF9F7;color:#18181B;font-family:-apple-system,sans-serif;padding:36px;line-height:1.6;'><h2>매직 링크가 만료되었거나 이미 사용되었습니다 · Magic Link Expired or Already Used</h2><p>텔레그램에서 <code>/dashboard</code> 를 입력하거나 터미널에서 <code>tuner --dashboard</code> 로 새 링크를 발급해 주세요.</p><p><a href='/dashboard/login' style='color:#18181B;font-weight:600;'>로그인 화면으로 이동 · Go to Sign In →</a></p></body></html>",
            ),
        )
            .into_response(),
    }
}

/// Request payload for `POST /dashboard/api/auth/login` and `POST /dashboard/api/auth/pin`.
#[derive(Deserialize)]
pub struct PinPayload {
    /// Owner PIN or passphrase.
    pub pin: String,
}

/// Verifies owner PIN (`POST /dashboard/api/auth/login`) with per-IP 5-strike lockout.
pub async fn handle_login(
    ext: Option<Extension<Arc<WebAppCtx>>>,
    peer: Option<ConnectInfo<SocketAddr>>,
    headers: HeaderMap,
    Json(payload): Json<PinPayload>,
) -> Response {
    let ctx = match resolve_ctx(ext) {
        Ok(c) => c,
        Err(r) => return *r,
    };
    let ip = extract_client_ip(&headers, peer.map(|c| c.0));
    if let Err(retry_after) = ctx.auth.check_login_rate_limit(ip).await {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(serde_json::json!({
                "error": format!("Too many failed login attempts. Retry in {}s.", retry_after),
                "retry_after": retry_after
            })),
        )
            .into_response();
    }
    if !ctx.auth.has_pin().await {
        return json_err(StatusCode::FORBIDDEN, "No owner PIN configured. Use a Magic Link first.");
    }
    if !ctx.auth.verify_pin(&payload.pin).await.unwrap_or(false) {
        ctx.auth.record_login_failure(ip).await;
        return json_err(StatusCode::UNAUTHORIZED, "Invalid PIN or passphrase");
    }
    ctx.auth.clear_login_failures(ip).await;
    let Ok(sess) = ctx.auth.create_session().await else {
        return json_err(StatusCode::INTERNAL_SERVER_ERROR, "Failed to create session");
    };
    let cookie = build_session_cookie(&sess.sid, 30 * 86_400, is_https_request(&headers));
    let mut res = Json(serde_json::json!({
        "authenticated": true,
        "csrf_token": sess.csrf_token,
        "profile": ctx.profile
    }))
    .into_response();
    if let Ok(hv) = header::HeaderValue::from_str(&cookie) {
        res.headers_mut().insert(header::SET_COOKIE, hv);
    }
    res
}

/// Returns current session status (`GET /dashboard/api/auth/me`).
pub async fn handle_auth_me(
    ext: Option<Extension<Arc<WebAppCtx>>>,
    headers: HeaderMap,
) -> Response {
    let ctx = match resolve_ctx(ext) {
        Ok(c) => c,
        Err(r) => return *r,
    };
    let has_pin = ctx.auth.has_pin().await;
    let tg_connected = ctx.bot.read().await.is_some();
    let bot_username = super::api_telegram::resolve_bot_username(&ctx).await;
    let lang = ctx.config.language.as_deref().unwrap_or("ko");
    let def_prov = crate::session::data::SessionData::effective_provider(&ctx.config.provider, "antigravity");
    let (any_ready, _) = super::provider_status::inspect_all_providers(&ctx.tuner_home);
    match require_auth(&ctx, &headers).await {
        Ok(sess) => Json(serde_json::json!({
            "authenticated": true,
            "csrf_token": sess.csrf_token,
            "has_pin": has_pin,
            "profile": ctx.profile,
            "language": lang,
            "telegram_connected": tg_connected,
            "bot_username": bot_username,
            "default_provider": def_prov,
            "default_model": ctx.config.model,
            "needs_onboarding": !any_ready
        }))
        .into_response(),
        Err(_) => (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "authenticated": false, "has_pin": has_pin, "language": lang })),
        )
            .into_response(),
    }
}

/// Sets or updates the owner PIN (`POST /dashboard/api/auth/pin`).
pub async fn handle_set_pin(
    ext: Option<Extension<Arc<WebAppCtx>>>,
    headers: HeaderMap,
    Json(payload): Json<PinPayload>,
) -> Response {
    let ctx = match resolve_ctx(ext) {
        Ok(c) => c,
        Err(r) => return *r,
    };
    if let Err(r) = require_mutation_auth(&ctx, &headers).await {
        return *r;
    }
    match ctx.auth.set_pin(&payload.pin).await {
        Ok(()) => Json(serde_json::json!({ "ok": true, "has_pin": true })).into_response(),
        Err(e) => json_err(StatusCode::BAD_REQUEST, &e),
    }
}

/// Logs out the current session (`POST /dashboard/api/auth/logout`).
pub async fn handle_logout(
    ext: Option<Extension<Arc<WebAppCtx>>>,
    headers: HeaderMap,
) -> Response {
    let ctx = match resolve_ctx(ext) {
        Ok(c) => c,
        Err(r) => return *r,
    };
    let sess = match require_mutation_auth(&ctx, &headers).await {
        Ok(s) => s,
        Err(r) => return *r,
    };
    let _ = ctx.auth.revoke_session(&sess.sid).await;
    let clear = build_session_cookie("", 0, is_https_request(&headers));
    let mut res = Json(serde_json::json!({ "ok": true })).into_response();
    if let Ok(hv) = header::HeaderValue::from_str(&clear) {
        res.headers_mut().insert(header::SET_COOKIE, hv);
    }
    res
}

/// Revokes all active sessions and pending Magic Links (`POST /dashboard/api/auth/revoke_all`).
pub async fn handle_revoke_all(
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
    let _ = ctx.auth.revoke_all().await;
    let clear = build_session_cookie("", 0, is_https_request(&headers));
    let mut res = Json(serde_json::json!({ "ok": true })).into_response();
    if let Ok(hv) = header::HeaderValue::from_str(&clear) {
        res.headers_mut().insert(header::SET_COOKIE, hv);
    }
    res
}
