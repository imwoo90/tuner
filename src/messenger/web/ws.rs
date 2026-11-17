//! # Web Dashboard WebSocket Upgrade & Event Pump (`ws.rs`)
//!
//! ## Overview
//! Authenticates `/dashboard/api/ws` via session cookie, `?csrf=` token comparison, and
//! strict `Origin` vs `Host` verification, then streams [`super::events::WebEvent`] frames.
//!
//! ## Search Tags
//! #websocket-auth, #origin-check, #ws-stream

use super::auth_http::{json_err, require_auth};
use super::{resolve_ctx, WebAppCtx};
use crate::security::vault_auth::proxy::constant_time_eq;
use axum::{
    extract::{
        ws::{Message as WsMsg, WebSocket, WebSocketUpgrade},
        Query,
    },
    http::{header, HeaderMap, StatusCode},
    response::Response,
    Extension,
};
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::broadcast::error::RecvError;

/// Validates that an optional `Origin` header matches the request `Host` header.
pub fn is_valid_ws_origin(headers: &HeaderMap) -> bool {
    let Some(origin_raw) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) else {
        return true;
    };
    let Some(host_raw) = headers.get(header::HOST).and_then(|v| v.to_str().ok()) else {
        return false;
    };
    let Ok(origin_url) = url::Url::parse(origin_raw.trim()) else {
        return false;
    };
    let Some(origin_host) = origin_url.host_str() else {
        return false;
    };
    let (host_name, host_port) = split_host_port(host_raw.trim());
    if !origin_host.eq_ignore_ascii_case(host_name) {
        return false;
    }
    let default_port = if origin_url.scheme().eq_ignore_ascii_case("https") {
        Some(443)
    } else {
        Some(80)
    };
    origin_url.port_or_known_default() == host_port.or(default_port)
}

fn split_host_port(host: &str) -> (&str, Option<u16>) {
    if let Some(rest) = host.strip_prefix('[')
        && let Some((addr, after)) = rest.split_once(']')
    {
        let port = after.strip_prefix(':').and_then(|p| p.parse::<u16>().ok());
        return (addr, port);
    }
    if let Some((h, p)) = host.rsplit_once(':')
        && let Ok(port) = p.parse::<u16>()
    {
        return (h, Some(port));
    }
    (host, None)
}

/// Query parameters for `GET /dashboard/api/ws`.
#[derive(Deserialize)]
pub struct WsAuthQuery {
    /// CSRF token matching the active session.
    #[serde(default)]
    pub csrf: String,
}

/// Authenticates and upgrades `/dashboard/api/ws` with cookie, `?csrf=`, and `Origin` checks.
pub async fn handle_ws(
    ext: Option<Extension<Arc<WebAppCtx>>>,
    headers: HeaderMap,
    Query(q): Query<WsAuthQuery>,
    ws: WebSocketUpgrade,
) -> Response {
    let ctx = match resolve_ctx(ext) {
        Ok(c) => c,
        Err(r) => return *r,
    };
    if !is_valid_ws_origin(&headers) {
        return json_err(StatusCode::FORBIDDEN, "WebSocket origin mismatch");
    }
    let sess = match require_auth(&ctx, &headers).await {
        Ok(s) => s,
        Err(r) => return *r,
    };
    if q.csrf.is_empty() || !constant_time_eq(q.csrf.as_bytes(), sess.csrf_token.as_bytes()) {
        return json_err(StatusCode::FORBIDDEN, "Invalid WebSocket CSRF token");
    }
    let sid = sess.sid;
    ws.on_upgrade(move |socket| pump_ws_events(socket, ctx, sid))
}

async fn pump_ws_events(socket: WebSocket, ctx: Arc<WebAppCtx>, sid: String) {
    let mut rx = ctx.events.subscribe();
    let (mut sender, mut receiver) = socket.split();
    let mut ping_iv = tokio::time::interval(std::time::Duration::from_secs(20));
    loop {
        tokio::select! {
            ev = rx.recv() => match ev {
                Ok(event) => {
                    if ctx.auth.verify_session(&sid).await.is_none() {
                        let _ = sender.send(WsMsg::Close(None)).await;
                        break;
                    }
                    if let Ok(txt) = serde_json::to_string(&event)
                        && sender.send(WsMsg::Text(txt)).await.is_err()
                    {
                        break;
                    }
                }
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => break,
            },
            _ = ping_iv.tick() => {
                if ctx.auth.verify_session(&sid).await.is_none()
                    || sender.send(WsMsg::Ping(Vec::new())).await.is_err()
                {
                    break;
                }
            }
            incoming = receiver.next() => match incoming {
                Some(Ok(WsMsg::Close(_))) | None => break,
                Some(Ok(WsMsg::Ping(p))) => {
                    let _ = sender.send(WsMsg::Pong(p)).await;
                }
                _ => {}
            }
        }
    }
}
