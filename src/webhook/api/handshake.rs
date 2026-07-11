//! # WebSocket Handshake
//!
//! Handles in-band authentication handshake for direct API sessions.

use crate::webhook::api::crypto::E2ESession;
use axum::extract::ws::Message;
use futures::sink::SinkExt;
use futures::stream::StreamExt;
use serde_json::Value;

pub async fn perform_handshake(
    sender: &mut futures::stream::SplitSink<axum::extract::ws::WebSocket, axum::extract::ws::Message>,
    receiver: &mut futures::stream::SplitStream<axum::extract::ws::WebSocket>,
    tok: &str,
    def_chat: i64,
    prov_info: &Value,
    getter: Option<&crate::webhook::api::websocket::ActiveStateGetter>,
) -> Result<(crate::session::key::SessionKey, E2ESession), String> {
    let auth_msg = match tokio::time::timeout(tokio::time::Duration::from_secs(10), receiver.next()).await {
        Ok(Some(Ok(msg))) => msg,
        _ => {
            send_reject(sender, "auth_timeout", "No auth message within 10 s").await;
            return Err("Auth timeout".to_string());
        }
    };
    let text = match auth_msg {
        axum::extract::ws::Message::Text(t) => t,
        _ => {
            send_reject(sender, "auth_required", "First message must be JSON text").await;
            return Err("First message not text".to_string());
        }
    };
    let data: Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(_) => {
            send_reject(sender, "auth_required", "First message must be auth JSON").await;
            return Err("Invalid JSON".to_string());
        }
    };
    let (raw_chat, chan_id, pk) = match validate_auth_msg(&data, tok) {
        Ok(res) => res,
        Err(e) => {
            send_reject(sender, "auth_failed", &e).await;
            return Err(e);
        }
    };
    let mut e2e = E2ESession::new();
    if e2e.set_remote_key(&pk).is_err() {
        send_reject(sender, "auth_failed", "Invalid e2e_pk").await;
        return Err("Invalid e2e_pk".to_string());
    }
    let chat_id = if raw_chat <= 0 { def_chat } else { raw_chat };
    let key = crate::session::key::SessionKey::for_transport("API", chat_id, chan_id);
    let ok_p = build_auth_ok(chat_id, chan_id, &e2e.local_pk_b64, prov_info, getter);
    let _ = sender.send(Message::Text(ok_p.to_string())).await;
    Ok((key, e2e))
}

fn validate_auth_msg(
    data: &Value,
    expected_token: &str,
) -> Result<(i64, Option<i64>, String), String> {
    if data.get("type").and_then(|t| t.as_str()) != Some("auth") {
        return Err("Not auth type".to_string());
    }
    let token = data.get("token").and_then(|t| t.as_str()).unwrap_or("");
    if !crate::webhook::auth::safe_compare(token.as_bytes(), expected_token.as_bytes()) {
        return Err("Invalid token".to_string());
    }
    let e2e_pk = data.get("e2e_pk").and_then(|t| t.as_str()).unwrap_or("");
    if e2e_pk.is_empty() {
        return Err("Missing e2e_pk".to_string());
    }
    let chat_id = data.get("chat_id").and_then(|v| v.as_i64()).unwrap_or(0);
    let channel_id = data.get("channel_id").and_then(|v| v.as_i64());
    Ok((chat_id, channel_id, e2e_pk.to_string()))
}

fn build_auth_ok(
    chat_id: i64,
    channel_id: Option<i64>,
    local_pk: &str,
    provider_info: &Value,
    active_state_getter: Option<&crate::webhook::api::websocket::ActiveStateGetter>,
) -> Value {
    let mut payload = serde_json::json!({
        "type": "auth_ok",
        "chat_id": chat_id,
        "e2e_pk": local_pk,
        "providers": provider_info,
    });
    if let Some(cid) = channel_id {
        payload
            .as_object_mut()
            .unwrap()
            .insert("channel_id".to_string(), serde_json::json!(cid));
    }
    if let Some(getter) = active_state_getter {
        let (active_provider, active_model) = getter();
        payload
            .as_object_mut()
            .unwrap()
            .insert("active_provider".to_string(), serde_json::json!(active_provider));
        payload
            .as_object_mut()
            .unwrap()
            .insert("active_model".to_string(), serde_json::json!(active_model));
    }
    payload
}

async fn send_reject(
    sender: &mut futures::stream::SplitSink<axum::extract::ws::WebSocket, axum::extract::ws::Message>,
    code: &str,
    message: &str,
) {
    let reject = serde_json::json!({
        "type": "error",
        "code": code,
        "message": message
    });
    let _ = sender.send(axum::extract::ws::Message::Text(reject.to_string())).await;
}
