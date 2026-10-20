//! # WebSocket Connection Negotiator
//!
//! ## Overview
//! Upgrades HTTP requests to WebSocket connections, validating tokens and establishing channel links.
//!
//! ## Search Tags
//! #websocket-upgrade, #channel-negotiator, #connection-handshake

use crate::webhook::api::handshake::perform_handshake;
use crate::webhook::api::server::ApiServerState;
use crate::webhook::api::session_loop::run_session_loop;
use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};
use futures::StreamExt;
use futures::sink::SinkExt;
use std::sync::Arc;

#[derive(Clone, Debug)]
/// Represents `ApiResult` (api result).
pub struct ApiResult {
    /// Text.
    pub text: String,
    /// Stream fallback.
    pub stream_fallback: bool,
}

#[async_trait::async_trait]
/// Defines the `ApiMessageHandler` (api message handler) interface.
pub trait ApiMessageHandler: Send + Sync {
    /// Executes `handle_message` (handle message).
    async fn handle_message(
        &self,
        key: crate::session::key::SessionKey,
        text: String,
        on_text_delta: Arc<dyn Fn(String) + Send + Sync>,
        on_tool_activity: Arc<dyn Fn(String) + Send + Sync>,
        on_system_status: Arc<dyn Fn(Option<String>) + Send + Sync>,
    ) -> Result<ApiResult, String>;
}

#[async_trait::async_trait]
/// Defines the `ApiAbortHandler` (api abort handler) interface.
pub trait ApiAbortHandler: Send + Sync {
    /// Executes `handle_abort` (handle abort).
    async fn handle_abort(&self, chat_id: i64) -> usize;
}

/// Type alias `ActiveStateGetter` (active state getter).
pub type ActiveStateGetter = Arc<dyn Fn() -> (String, String) + Send + Sync>;

/// Executes `handle_health` (handle health).
pub async fn handle_health(
    State(state): State<Arc<std::sync::Mutex<ApiServerState>>>,
) -> impl IntoResponse {
    let connections = {
        let s = state.lock().unwrap_or_else(|e| e.into_inner());
        s.active_ws.lock().unwrap_or_else(|e| e.into_inner()).len()
    };
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "connections": connections,
        })),
    )
}

/// Executes `handle_websocket` (handle websocket).
pub async fn handle_websocket(
    ws: axum::extract::WebSocketUpgrade,
    State(state): State<Arc<std::sync::Mutex<ApiServerState>>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ws_session(socket, state))
}

async fn handle_ws_session(
    socket: axum::extract::ws::WebSocket,
    state: Arc<std::sync::Mutex<ApiServerState>>,
) {
    let (mut sender, mut receiver) = socket.split();

    let (token, default_chat_id, provider_info, getter) = {
        let s = state.lock().unwrap_or_else(|e| e.into_inner());
        (
            s.config.token.clone(),
            s.default_chat_id,
            s.provider_info.clone(),
            s.active_state_getter.clone(),
        )
    };

    let handshake_res = perform_handshake(
        &mut sender,
        &mut receiver,
        &token,
        default_chat_id,
        &provider_info,
        getter.as_ref(),
    )
    .await;

    let Ok((key, e2e)) = handshake_res else {
        return;
    };

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<axum::extract::ws::Message>();
    let conn_id = {
        let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
        s.next_conn_id += 1;
        let id = s.next_conn_id;
        s.active_ws.lock().unwrap_or_else(|e| e.into_inner()).insert(id, tx.clone());
        id
    };

    let sender = Arc::new(tokio::sync::Mutex::new(sender));
    let sender_clone = sender.clone();
    let forward_task = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            let mut s = sender_clone.lock().await;
            if s.send(msg).await.is_err() {
                break;
            }
        }
    });

    run_session_loop(key, e2e, receiver, tx.clone(), state.clone()).await;

    {
        let active_ws = { state.lock().unwrap_or_else(|e| e.into_inner()).active_ws.clone() };
        active_ws.lock().unwrap_or_else(|e| e.into_inner()).remove(&conn_id);
    }
    forward_task.abort();
}
