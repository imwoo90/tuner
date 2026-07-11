//! # WebSocket Session Loop
//!
//! Handles decrypted message routing.

use crate::webhook::api::crypto::E2ESession;
use crate::webhook::api::server::ApiServerState;
use axum::extract::ws::Message;
use futures::stream::StreamExt;
use serde_json::Value;
use std::sync::Arc;

pub async fn run_session_loop(
    key: crate::session::key::SessionKey,
    e2e: E2ESession,
    mut receiver: futures::stream::SplitStream<axum::extract::ws::WebSocket>,
    tx: tokio::sync::mpsc::UnboundedSender<axum::extract::ws::Message>,
    state: Arc<std::sync::Mutex<ApiServerState>>,
) {
    let tx_val = tx.clone();
    let e2e_arc = Arc::new(e2e);
    let e2e_val = e2e_arc.clone();
    let on_text_delta: Arc<dyn Fn(String) + Send + Sync> = Arc::new(move |delta: String| {
        send_encrypted_response(&tx_val, &e2e_val, &serde_json::json!({ "type": "text_delta", "data": delta }));
    });

    let tx_val = tx.clone();
    let e2e_val = e2e_arc.clone();
    let on_tool_activity: Arc<dyn Fn(String) + Send + Sync> = Arc::new(move |name: String| {
        send_encrypted_response(&tx_val, &e2e_val, &serde_json::json!({ "type": "tool_activity", "data": name }));
    });

    let tx_val = tx.clone();
    let e2e_val = e2e_arc.clone();
    let on_system_status: Arc<dyn Fn(Option<String>) + Send + Sync> = Arc::new(move |label: Option<String>| {
        send_encrypted_response(&tx_val, &e2e_val, &serde_json::json!({ "type": "system_status", "data": label }));
    });

    while let Some(Ok(msg)) = receiver.next().await {
        handle_session_frame(
            msg,
            &key,
            &e2e_arc,
            &tx,
            &state,
            &on_text_delta,
            &on_tool_activity,
            &on_system_status,
        )
        .await;
    }
}

async fn handle_session_frame(
    msg: Message,
    key: &crate::session::key::SessionKey,
    e2e: &E2ESession,
    tx: &tokio::sync::mpsc::UnboundedSender<Message>,
    state: &Arc<std::sync::Mutex<ApiServerState>>,
    on_text_delta: &Arc<dyn Fn(String) + Send + Sync>,
    on_tool_activity: &Arc<dyn Fn(String) + Send + Sync>,
    on_system_status: &Arc<dyn Fn(Option<String>) + Send + Sync>,
) {
    if let axum::extract::ws::Message::Text(encrypted_frame) = msg {
        let decrypted = match e2e.decrypt(&encrypted_frame) {
            Ok(val) => val,
            Err(_) => {
                send_encrypted_error(tx, e2e, "decrypt_failed", "Decryption failed");
                return;
            }
        };

        let msg_type = decrypted.get("type").and_then(|t| t.as_str()).unwrap_or("");
        if msg_type == "message" {
            let text = decrypted.get("text").and_then(|t| t.as_str()).unwrap_or("").trim().to_string();
            handle_message_request(text, key, e2e, tx, state, on_text_delta, on_tool_activity, on_system_status).await;
        } else if msg_type == "abort" {
            let killed = handle_abort_helper(state, key.chat_id).await;
            send_encrypted_response(tx, e2e, &serde_json::json!({ "type": "abort_ok", "killed": killed }));
        } else {
            send_encrypted_error(tx, e2e, "unknown_type", &format!("Unknown message type: {}", msg_type));
        }
    }
}

async fn handle_message_request(
    text: String,
    key: &crate::session::key::SessionKey,
    e2e: &E2ESession,
    tx: &tokio::sync::mpsc::UnboundedSender<Message>,
    state: &Arc<std::sync::Mutex<ApiServerState>>,
    otd: &Arc<dyn Fn(String) + Send + Sync>,
    ota: &Arc<dyn Fn(String) + Send + Sync>,
    oss: &Arc<dyn Fn(Option<String>) + Send + Sync>,
) {
    if text.is_empty() {
        send_encrypted_error(tx, e2e, "empty", "Empty message");
        return;
    }
    if text.to_lowercase() == "/stop" {
        let killed = handle_abort_helper(state, key.chat_id).await;
        send_encrypted_response(tx, e2e, &serde_json::json!({ "type": "abort_ok", "killed": killed }));
        return;
    }
    let lock = {
        let s = state.lock().unwrap();
        s.lock_pool.get((key.chat_id, key.topic_id))
    };
    let _guard = lock.lock().await;
    let opt_msg_handler = { state.lock().unwrap().message_handler.clone() };
    if let Some(ref handler) = opt_msg_handler {
        match handler.handle_message(key.clone(), text, otd.clone(), ota.clone(), oss.clone()).await {
            Ok(res) => {
                let files = crate::webhook::api::files::parse_file_refs(&res.text);
                send_encrypted_response(tx, e2e, &serde_json::json!({
                    "type": "result", "text": res.text, "stream_fallback": res.stream_fallback, "files": files,
                }));
            }
            Err(_) => {
                send_encrypted_error(tx, e2e, "internal_error", "An internal error occurred");
            }
        }
    } else {
        send_encrypted_error(tx, e2e, "no_handler", "Message handler not configured");
    }
}

fn send_encrypted_error(
    tx: &tokio::sync::mpsc::UnboundedSender<axum::extract::ws::Message>,
    e2e: &E2ESession,
    code: &str,
    msg: &str,
) {
    let frame = serde_json::json!({
        "type": "error",
        "code": code,
        "message": msg
    });
    if let Ok(enc) = e2e.encrypt(&frame) {
        let _ = tx.send(axum::extract::ws::Message::Text(enc));
    }
}

fn send_encrypted_response(
    tx: &tokio::sync::mpsc::UnboundedSender<axum::extract::ws::Message>,
    e2e: &E2ESession,
    data: &Value,
) {
    if let Ok(enc) = e2e.encrypt(data) {
        let _ = tx.send(axum::extract::ws::Message::Text(enc));
    }
}

async fn handle_abort_helper(state: &Arc<std::sync::Mutex<ApiServerState>>, chat_id: i64) -> usize {
    let opt_abort = { state.lock().unwrap().abort_handler.clone() };
    if let Some(ref abort) = opt_abort {
        abort.handle_abort(chat_id).await
    } else {
        0
    }
}
