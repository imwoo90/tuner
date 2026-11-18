//! # Public `:8743` Viewer HTTP & WebSocket Reverse Proxy (`proxy_relay.rs`)
//!
//! ## Overview
//! Serves the public token-gated `/auth/:token` HTML page and `/auth/:token/ws` binary JPEG
//! WebSocket relay on the warm `:8743` [`crate::messenger::telegram::review_server`] router,
//! forwarding strictly to verified `127.0.0.1` ports registered in [`super::proxy`].
//!
//! ## Search Tags
//! #vault-auth-relay, #websocket-proxy, #review-server-auth

use super::proxy::resolve_route_port;
use axum::{
    extract::{
        ws::{Message as AxumWsMsg, WebSocket, WebSocketUpgrade},
        Path,
    },
    http::{header, StatusCode},
    response::{Html, IntoResponse, Response},
    routing::get,
    Router,
};
use futures::{SinkExt, StreamExt};
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message as TungMsg;

/// Builds ONLY the public token-gated viewer routes merged into `:8743` `review_server`.
pub fn auth_proxy_routes() -> Router {
    Router::new()
        .route("/auth/:token", get(handle_proxy_page))
        .route("/auth/:token/ws", get(handle_proxy_ws))
}

async fn handle_proxy_page(Path(token): Path<String>) -> Response {
    let Some(port) = resolve_route_port(&token).await else {
        return (
            StatusCode::NOT_FOUND,
            "Remote auth session expired or completed.",
        )
            .into_response();
    };
    let url = format!("http://127.0.0.1:{port}/auth/{token}");
    let Ok(client) = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(5))
        .build()
    else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    match client.get(&url).send().await {
        Ok(resp) if resp.status().is_success() => {
            let body = resp.text().await.unwrap_or_default();
            let mut out = Html(body).into_response();
            if let Ok(hv) = header::HeaderValue::from_str("no-store, no-cache, must-revalidate") {
                out.headers_mut().insert(header::CACHE_CONTROL, hv);
            }
            if let Ok(hv) = header::HeaderValue::from_str("SAMEORIGIN") {
                out.headers_mut().insert(header::X_FRAME_OPTIONS, hv);
            }
            out
        }
        Ok(resp) => StatusCode::from_u16(resp.status().as_u16())
            .unwrap_or(StatusCode::BAD_GATEWAY)
            .into_response(),
        Err(_) => (StatusCode::BAD_GATEWAY, "Upstream auth server closed").into_response(),
    }
}

async fn handle_proxy_ws(Path(token): Path<String>, ws: WebSocketUpgrade) -> Response {
    let Some(port) = resolve_route_port(&token).await else {
        return (StatusCode::NOT_FOUND, "Remote auth session expired").into_response();
    };
    ws.max_message_size(8 * 1024 * 1024)
        .on_upgrade(move |client_ws| relay_websocket(client_ws, port, token))
}

async fn relay_websocket(client_ws: WebSocket, port: u16, token: String) {
    let upstream_url = format!("ws://127.0.0.1:{port}/auth/{token}/ws");
    let Ok((upstream_ws, _)) = tokio_tungstenite::connect_async(&upstream_url).await else {
        return;
    };
    let (mut client_tx, mut client_rx) = client_ws.split();
    let (mut up_tx, mut up_rx) = upstream_ws.split();

    let c2u = async {
        while let Some(Ok(msg)) = client_rx.next().await {
            let mapped = match msg {
                AxumWsMsg::Text(t) => TungMsg::Text(t.to_string()),
                AxumWsMsg::Binary(b) => TungMsg::Binary(b.to_vec()),
                AxumWsMsg::Ping(p) => TungMsg::Ping(p.to_vec()),
                AxumWsMsg::Pong(p) => TungMsg::Pong(p.to_vec()),
                AxumWsMsg::Close(_) => break,
            };
            if up_tx.send(mapped).await.is_err() {
                break;
            }
        }
    };

    let u2c = async {
        while let Some(Ok(msg)) = up_rx.next().await {
            let mapped = match msg {
                TungMsg::Text(t) => AxumWsMsg::Text(t),
                TungMsg::Binary(b) => AxumWsMsg::Binary(b),
                TungMsg::Ping(p) => AxumWsMsg::Ping(p),
                TungMsg::Pong(p) => AxumWsMsg::Pong(p),
                TungMsg::Close(_) => break,
                TungMsg::Frame(_) => continue,
            };
            if client_tx.send(mapped).await.is_err() {
                break;
            }
        }
    };

    tokio::select! {
        _ = c2u => {}
        _ = u2c => {}
    }
}
