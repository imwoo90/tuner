//! # WebSocket Connection Negotiator
//!
//! ## Overview
//! Upgrades HTTP requests to WebSocket connections, validating tokens and establishing channel links.
//!
//! ## Collaboration Graph
//! - Invokes [`session_loop::run_session_loop`](super::session_loop::run_session_loop) on handshake success.
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
pub struct ApiResult {
    pub text: String,
    pub stream_fallback: bool,
}

#[async_trait::async_trait]
pub trait ApiMessageHandler: Send + Sync {
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
pub trait ApiAbortHandler: Send + Sync {
    async fn handle_abort(&self, chat_id: i64) -> usize;
}

pub type ActiveStateGetter = Arc<dyn Fn() -> (String, String) + Send + Sync>;

pub async fn handle_health(
    State(state): State<Arc<std::sync::Mutex<ApiServerState>>>,
) -> impl IntoResponse {
    let connections = {
        let s = state.lock().unwrap();
        s.active_ws.lock().unwrap().len()
    };
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "connections": connections,
        })),
    )
}

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
        let s = state.lock().unwrap();
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
        let mut s = state.lock().unwrap();
        s.next_conn_id += 1;
        let id = s.next_conn_id;
        s.active_ws.lock().unwrap().insert(id, tx.clone());
        id
    };

    let sender = Arc::new(tokio::sync::Mutex::new(sender));
    let sender_clone = sender.clone();
    let forward_task = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            let mut s = sender_clone.lock().await;
            if let Err(_) = s.send(msg).await {
                break;
            }
        }
    });

    run_session_loop(key, e2e, receiver, tx.clone(), state.clone()).await;

    {
        let active_ws = { state.lock().unwrap().active_ws.clone() };
        active_ws.lock().unwrap().remove(&conn_id);
    }
    forward_task.abort();
}
