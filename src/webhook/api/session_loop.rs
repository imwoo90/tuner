//! # Remote Session WebSocket Loop
//!
//! ## Overview
//! Coordinates bidirectional command streams, handling remote terminal standard I/O channels over
//! websocket sockets.
//!
//! ## Search Tags
//! #websocket-loop, #remote-stdin, #full-duplex-stream

use crate::webhook::api::crypto::E2ESession;
use crate::webhook::api::server::ApiServerState;
use axum::extract::ws::Message;
use futures::stream::StreamExt;
use serde_json::Value;
use std::sync::Arc;

#[derive(Clone)]
struct SessionLoopCtx {
    key: crate::session::key::SessionKey,
    e2e: Arc<E2ESession>,
    tx: tokio::sync::mpsc::UnboundedSender<Message>,
    state: Arc<std::sync::Mutex<ApiServerState>>,
    on_text_delta: Arc<dyn Fn(String) + Send + Sync>,
    on_tool_activity: Arc<dyn Fn(String) + Send + Sync>,
    on_system_status: Arc<dyn Fn(Option<String>) + Send + Sync>,
}

/// Executes `run_session_loop` (run session loop).
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
        send_encrypted_response(
            &tx_val,
            &e2e_val,
            &serde_json::json!({ "type": "text_delta", "data": delta }),
        );
    });

    let tx_val = tx.clone();
    let e2e_val = e2e_arc.clone();
    let on_tool_activity: Arc<dyn Fn(String) + Send + Sync> = Arc::new(move |name: String| {
        send_encrypted_response(
            &tx_val,
            &e2e_val,
            &serde_json::json!({ "type": "tool_activity", "data": name }),
        );
    });

    let tx_val = tx.clone();
    let e2e_val = e2e_arc.clone();
    let on_system_status: Arc<dyn Fn(Option<String>) + Send + Sync> =
        Arc::new(move |label: Option<String>| {
            send_encrypted_response(
                &tx_val,
                &e2e_val,
                &serde_json::json!({ "type": "system_status", "data": label }),
            );
        });

    let ctx = SessionLoopCtx {
        key,
        e2e: e2e_arc,
        tx,
        state,
        on_text_delta,
        on_tool_activity,
        on_system_status,
    };

    let mut active_task: Option<tokio::task::JoinHandle<()>> = None;

    while let Some(Ok(msg)) = receiver.next().await {
        if let Some(ref h) = active_task
            && h.is_finished()
        {
            active_task = None;
        }
        handle_session_frame(msg, &ctx, &mut active_task).await;
    }
}

async fn handle_message_frame(
    decrypted: Value,
    ctx: &SessionLoopCtx,
    active_task: &mut Option<tokio::task::JoinHandle<()>>,
) {
    let text = decrypted
        .get("text")
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if text.to_lowercase() == "/stop" {
        if let Some(h) = active_task.take() {
            h.abort();
        }
        let killed = handle_abort_helper(&ctx.state, ctx.key.chat_id).await;
        send_encrypted_response(
            &ctx.tx,
            &ctx.e2e,
            &serde_json::json!({ "type": "abort_ok", "killed": killed }),
        );
        return;
    }
    if text.is_empty() {
        send_encrypted_error(&ctx.tx, &ctx.e2e, "empty", "Empty message");
        return;
    }

    if let Some(h) = active_task.take() {
        h.abort();
    }

    let ctx_clone = ctx.clone();
    let handle = tokio::spawn(async move {
        handle_message_request(text, &ctx_clone).await;
    });
    *active_task = Some(handle);
}

async fn handle_session_frame(
    msg: Message,
    ctx: &SessionLoopCtx,
    active_task: &mut Option<tokio::task::JoinHandle<()>>,
) {
    if let axum::extract::ws::Message::Text(encrypted_frame) = msg {
        let decrypted = match ctx.e2e.decrypt(&encrypted_frame) {
            Ok(val) => val,
            Err(_) => {
                send_encrypted_error(&ctx.tx, &ctx.e2e, "decrypt_failed", "Decryption failed");
                return;
            }
        };

        let msg_type = decrypted.get("type").and_then(|t| t.as_str()).unwrap_or("");
        if msg_type == "message" {
            handle_message_frame(decrypted, ctx, active_task).await;
        } else if msg_type == "abort" {
            if let Some(h) = active_task.take() {
                h.abort();
            }
            let killed = handle_abort_helper(&ctx.state, ctx.key.chat_id).await;
            send_encrypted_response(
                &ctx.tx,
                &ctx.e2e,
                &serde_json::json!({ "type": "abort_ok", "killed": killed }),
            );
        } else {
            send_encrypted_error(
                &ctx.tx,
                &ctx.e2e,
                "unknown_type",
                &format!("Unknown message type: {}", msg_type),
            );
        }
    }
}

async fn handle_message_request(text: String, ctx: &SessionLoopCtx) {
    if text.is_empty() {
        send_encrypted_error(&ctx.tx, &ctx.e2e, "empty", "Empty message");
        return;
    }
    if text.to_lowercase() == "/stop" {
        let killed = handle_abort_helper(&ctx.state, ctx.key.chat_id).await;
        send_encrypted_response(
            &ctx.tx,
            &ctx.e2e,
            &serde_json::json!({ "type": "abort_ok", "killed": killed }),
        );
        return;
    }
    let lock = {
        let s = ctx.state.lock().unwrap_or_else(|e| e.into_inner());
        s.lock_pool.get((ctx.key.chat_id, ctx.key.topic_id))
    };
    let _guard = lock.lock().await;
    let opt_msg_handler = {
        ctx.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .message_handler
            .clone()
    };
    if let Some(ref handler) = opt_msg_handler {
        match handler
            .handle_message(
                ctx.key.clone(),
                text,
                ctx.on_text_delta.clone(),
                ctx.on_tool_activity.clone(),
                ctx.on_system_status.clone(),
            )
            .await
        {
            Ok(res) => {
                let files = crate::webhook::api::files::parse_file_refs(&res.text);
                send_encrypted_response(
                    &ctx.tx,
                    &ctx.e2e,
                    &serde_json::json!({
                        "type": "result", "text": res.text, "stream_fallback": res.stream_fallback, "files": files,
                    }),
                );
            }
            Err(_) => {
                send_encrypted_error(&ctx.tx, &ctx.e2e, "internal_error", "An internal error occurred");
            }
        }
    } else {
        send_encrypted_error(&ctx.tx, &ctx.e2e, "no_handler", "Message handler not configured");
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
    let opt_abort = { state.lock().unwrap_or_else(|e| e.into_inner()).abort_handler.clone() };
    if let Some(ref abort) = opt_abort {
        abort.handle_abort(chat_id).await
    } else {
        0
    }
}
