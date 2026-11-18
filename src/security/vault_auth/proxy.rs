//! # Loopback IPC Registry for Warm Tunnel Reuse (`proxy.rs`)
//!
//! ## Overview
//! Registers short-lived `tuner --vault auth` loopback ports with the long-running `:8743`
//! worker via an isolated `127.0.0.1:0` IPC listener (never exposed on `:8743` or Cloudflare),
//! enforcing constant-time 256-bit secret checks, worker PID liveness, strict 64-hex tokens,
//! and upstream `/auth/:token/ping` SSRF verification before forwarding via [`super::proxy_relay`].
//!
//! ## Search Tags
//! #vault-auth-proxy, #warm-tunnel-reuse, #loopback-ipc, #ssrf-guard

pub use super::proxy_relay::auth_proxy_routes;

use crate::security::vault::current_unix_secs;
use axum::{
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path as StdPath;
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::sync::Mutex;

const IPC_HEADER: &str = "x-tuner-ipc-secret";
const ENDPOINT_FILE: &str = "tunnel_endpoint.json";
/// Exact handshake body returned by `GET /auth/:token/ping` on a genuine local HITL server.
pub const HITL_PING_MAGIC: &str = "TUNER_HITL_OK";

/// Persisted loopback IPC descriptor written by the running `:8743` worker (`chmod 0600`).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TunnelEndpointInfo {
    /// Local TCP port bound by `review_server` (typically `8743`).
    pub port: u16,
    /// Dedicated `127.0.0.1` loopback-only TCP port serving `/internal/auth/*`.
    pub ipc_port: u16,
    /// OS PID of the worker that published this descriptor.
    pub pid: u32,
    /// Per-process 256-bit random IPC secret required on `/internal/auth/*`.
    pub ipc_secret: String,
}

#[derive(Clone, Debug)]
struct ProxyRouteEntry {
    local_port: u16,
    expires_at_secs: u64,
    site_key: String,
}

/// Active HITL remote browser session descriptor exposed to authenticated Dashboard clients.
#[derive(Clone, Debug, Serialize)]
pub struct ActiveAuthSessionInfo {
    /// One-time 64-hex session token.
    pub token: String,
    /// Canonical site key slug (`github`, `naver`, etc.).
    pub site_key: String,
    /// Remaining session TTL in seconds.
    pub remaining_secs: u64,
    /// Relative same-origin viewer path (`/auth/<token>`).
    pub viewer_path: String,
}

/// Request payload for `POST /internal/auth/register`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RegisterAuthRouteReq {
    /// One-time 256-bit hex session token (`64` hex chars).
    pub token: String,
    /// Ephemeral `127.0.0.1` port hosted by `tuner --vault auth`.
    pub local_port: u16,
    /// Expiration Unix timestamp in seconds.
    pub expires_at_secs: u64,
    /// Skip spawning a new Quick Tunnel when called from an active `:8743` Dashboard session.
    #[serde(default)]
    pub skip_quick_tunnel: bool,
}

/// Request payload for `POST /internal/auth/unregister`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UnregisterAuthRouteReq {
    /// One-time 256-bit hex session token to remove.
    pub token: String,
}

struct AuthProxyRegistry {
    ipc_secret: Mutex<String>,
    ipc_port: Mutex<Option<u16>>,
    routes: Mutex<HashMap<String, ProxyRouteEntry>>,
}

static REGISTRY: OnceLock<Arc<AuthProxyRegistry>> = OnceLock::new();

fn registry() -> &'static Arc<AuthProxyRegistry> {
    REGISTRY.get_or_init(|| {
        Arc::new(AuthProxyRegistry {
            ipc_secret: Mutex::new(hex::encode(rand::random::<[u8; 32]>())),
            ipc_port: Mutex::new(None),
            routes: Mutex::new(HashMap::new()),
        })
    })
}

/// Compares two byte slices in constant time without early-exit length/byte leakage.
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() || a.is_empty() {
        return false;
    }
    let mut diff = 0u8;
    for (&x, &y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Validates that `token` is strictly a 64-character ASCII hexadecimal 256-bit token.
pub fn is_strict_hex_token(token: &str) -> bool {
    token.len() == 64 && token.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Builds the dedicated `127.0.0.1`-only IPC router for route registration and removal.
pub fn build_ipc_router() -> Router {
    Router::new()
        .route("/internal/auth/register", post(handle_register))
        .route("/internal/auth/unregister", post(handle_unregister))
}

async fn ensure_ipc_server_running() -> Option<u16> {
    let mut guard = registry().ipc_port.lock().await;
    if let Some(p) = *guard {
        if tokio::net::TcpStream::connect(("127.0.0.1", p)).await.is_ok() {
            return Some(p);
        }
        *guard = None;
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.ok()?;
    let port = listener.local_addr().ok()?.port();
    *guard = Some(port);
    let app = build_ipc_router();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Some(port)
}

/// Spawns the `127.0.0.1` IPC listener and publishes `0600` `tunnel_endpoint.json` in `vault_dir`.
pub async fn publish_tunnel_endpoint(vault_dir: &StdPath, port: u16) {
    let Some(ipc_port) = ensure_ipc_server_running().await else {
        return;
    };
    let _ = std::fs::create_dir_all(vault_dir);
    let secret = registry().ipc_secret.lock().await.clone();
    let info = TunnelEndpointInfo {
        port,
        ipc_port,
        pid: std::process::id(),
        ipc_secret: secret,
    };
    let Ok(raw) = serde_json::to_string(&info) else {
        return;
    };
    let tmp = vault_dir.join(".tunnel_endpoint.json.tmp");
    let dst = vault_dir.join(ENDPOINT_FILE);
    if std::fs::write(&tmp, raw).is_ok() {
        set_mode_0600(&tmp);
        let _ = std::fs::rename(&tmp, &dst);
        set_mode_0600(&dst);
    }
}

/// Reads `tunnel_endpoint.json` from `vault_dir` if owned by a live worker PID.
pub fn load_tunnel_endpoint(vault_dir: &StdPath) -> Option<TunnelEndpointInfo> {
    let raw = std::fs::read_to_string(vault_dir.join(ENDPOINT_FILE)).ok()?;
    let info: TunnelEndpointInfo = serde_json::from_str(&raw).ok()?;
    if info.ipc_port == 0 || info.pid == 0 {
        return None;
    }
    #[cfg(target_os = "linux")]
    if !StdPath::new(&format!("/proc/{}", info.pid)).exists() {
        return None;
    }
    Some(info)
}

/// Returns `true` if any active HITL remote browser auth session is registered on `:8743`.
pub async fn has_active_auth_sessions() -> bool {
    let now = current_unix_secs();
    let mut guard = registry().routes.lock().await;
    guard.retain(|_, v| v.expires_at_secs > now);
    !guard.is_empty()
}

/// Lists all active non-expired HITL remote browser sessions registered on `:8743`.
pub async fn list_active_auth_sessions() -> Vec<ActiveAuthSessionInfo> {
    let now = current_unix_secs();
    let mut guard = registry().routes.lock().await;
    guard.retain(|_, v| v.expires_at_secs > now);
    let mut out: Vec<ActiveAuthSessionInfo> = guard
        .iter()
        .map(|(tok, e)| ActiveAuthSessionInfo {
            token: tok.clone(),
            site_key: e.site_key.clone(),
            remaining_secs: e.expires_at_secs.saturating_sub(now),
            viewer_path: format!("/auth/{tok}"),
        })
        .collect();
    out.sort_by_key(|b| std::cmp::Reverse(b.remaining_secs));
    out
}

pub(crate) async fn resolve_route_port(token: &str) -> Option<u16> {
    if !is_strict_hex_token(token) {
        return None;
    }
    let now = current_unix_secs();
    let mut guard = registry().routes.lock().await;
    guard.retain(|_, v| v.expires_at_secs > now);
    guard.get(token).map(|e| e.local_port)
}

async fn verify_ipc_header(headers: &HeaderMap) -> bool {
    let Some(val) = headers.get(IPC_HEADER).and_then(|h| h.to_str().ok()) else {
        return false;
    };
    let expected = registry().ipc_secret.lock().await.clone();
    constant_time_eq(val.as_bytes(), expected.as_bytes())
}

async fn verify_upstream_hitl_ping(local_port: u16, token: &str) -> Option<String> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(2))
        .build()
        .ok()?;
    let url = format!("http://127.0.0.1:{local_port}/auth/{token}/ping");
    let resp = client.get(&url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let site_key = resp
        .headers()
        .get("x-tuner-site-key")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("browser")
        .to_string();
    let body = resp.text().await.ok()?;
    (body.trim() == HITL_PING_MAGIC).then_some(site_key)
}

async fn handle_register(
    headers: HeaderMap,
    Json(req): Json<RegisterAuthRouteReq>,
) -> Response {
    if !verify_ipc_header(&headers).await {
        return (StatusCode::FORBIDDEN, "Invalid IPC secret").into_response();
    }
    if !is_strict_hex_token(&req.token) || req.local_port == 0 {
        return (StatusCode::BAD_REQUEST, "Invalid route token or port").into_response();
    }
    let Some(site_key) = verify_upstream_hitl_ping(req.local_port, &req.token).await else {
        return (
            StatusCode::BAD_REQUEST,
            "Upstream port failed HITL ping verification",
        )
            .into_response();
    };
    {
        let now = current_unix_secs();
        let mut guard = registry().routes.lock().await;
        guard.retain(|_, v| v.expires_at_secs > now);
        guard.insert(
            req.token.clone(),
            ProxyRouteEntry {
                local_port: req.local_port,
                expires_at_secs: req.expires_at_secs,
                site_key,
            },
        );
    }
    crate::messenger::web::events_bridge::hitl_updated(true).await;
    let mgr = crate::messenger::telegram::review::global_review_manager();
    let public_url = if req.skip_quick_tunnel {
        let port = mgr.ensure_server_running().await;
        match mgr.current_tunnel_url().await {
            Some(base) => format!("{base}/auth/{}", req.token),
            None => format!("http://127.0.0.1:{port}/auth/{}", req.token),
        }
    } else {
        mgr.get_auth_url(&req.token).await
    };
    Json(serde_json::json!({ "public_url": public_url })).into_response()
}

async fn handle_unregister(
    headers: HeaderMap,
    Json(req): Json<UnregisterAuthRouteReq>,
) -> Response {
    if !verify_ipc_header(&headers).await {
        return (StatusCode::FORBIDDEN, "Invalid IPC secret").into_response();
    }
    if !is_strict_hex_token(&req.token) {
        return (StatusCode::BAD_REQUEST, "Invalid token").into_response();
    }
    let (removed, still_active) = {
        let mut g = registry().routes.lock().await;
        let rem = g.remove(&req.token).is_some();
        (rem, !g.is_empty())
    };
    crate::messenger::web::events_bridge::hitl_updated(still_active).await;
    Json(serde_json::json!({ "removed": removed })).into_response()
}

pub(crate) fn set_mode_0600(path: &StdPath) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(path) {
            let mut perms = meta.permissions();
            perms.set_mode(0o600);
            let _ = std::fs::set_permissions(path, perms);
        }
    }
}

pub(crate) fn set_mode_0700(path: &StdPath) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(path) {
            let mut perms = meta.permissions();
            perms.set_mode(0o700);
            let _ = std::fs::set_permissions(path, perms);
        }
    }
}
