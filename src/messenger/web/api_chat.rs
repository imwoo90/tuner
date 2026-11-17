//! # Web Dashboard Chat Send, Stop & New Endpoints (`api_chat.rs`)
//!
//! ## Overview
//! Handles Web-initiated prompt dispatch, soft `/stop` interrupts across active
//! provider pools, and `/new` clean handover resets.
//!
//! ## Search Tags
//! #web-chat, #soft-stop, #session-reset

use super::auth_http::{json_err, require_mutation_auth};
use super::events::{emit_session_updated, WebEvent};
use super::turn_boot::ensure_web_session_ready_for_turn;
use super::turn_runner::execute_web_turn;
use super::{resolve_ctx, WebAppCtx};
use crate::session::{data::SessionData, key::SessionKey};
use axum::{
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Extension, Json,
};
use serde::Deserialize;
use std::sync::Arc;

/// Request payload for `POST /dashboard/api/sessions/send`.
#[derive(Deserialize)]
pub struct SendPromptPayload {
    /// Target session key (`tg:...` or `web:0:...`).
    pub session_key: String,
    /// Prompt text to send.
    #[serde(default)]
    pub text: String,
    /// Optional explicit attachment paths under `telegram_files/`.
    #[serde(default)]
    pub attachments: Vec<String>,
}

/// Request payload for `POST /dashboard/api/sessions/stop` and `/new`.
#[derive(Deserialize)]
pub struct SessionKeyPayload {
    /// Target session key.
    pub session_key: String,
}

/// Enqueues a user prompt turn (`POST /dashboard/api/sessions/send`).
pub async fn handle_send_prompt(
    ext: Option<Extension<Arc<WebAppCtx>>>,
    headers: HeaderMap,
    Json(payload): Json<SendPromptPayload>,
) -> Response {
    let ctx = match resolve_ctx(ext) {
        Ok(c) => c,
        Err(r) => return *r,
    };
    if let Err(r) = require_mutation_auth(&ctx, &headers).await {
        return *r;
    }
    let Ok(key) = payload.session_key.parse::<SessionKey>() else {
        return json_err(StatusCode::BAD_REQUEST, "Invalid session_key");
    };
    let Ok(Some(sess)) = ctx.sessions.get_active(&key).await else {
        return json_err(StatusCode::NOT_FOUND, "Session not found");
    };
    let trimmed = payload.text.trim();
    if trimmed.is_empty() && payload.attachments.is_empty() && sess.pending_attachments.is_empty() {
        return json_err(StatusCode::BAD_REQUEST, "Prompt text or attachment required");
    }
    let ctx_clone = ctx.clone();
    let sk = key.storage_key();
    tokio::spawn(async move {
        execute_web_turn(ctx_clone, key, payload.text, payload.attachments).await;
    });
    (StatusCode::ACCEPTED, Json(serde_json::json!({ "ok": true, "session_key": sk })))
        .into_response()
}

/// Soft-interrupts active CLI execution (`POST /dashboard/api/sessions/stop`).
pub async fn handle_stop_session(
    ext: Option<Extension<Arc<WebAppCtx>>>,
    headers: HeaderMap,
    Json(payload): Json<SessionKeyPayload>,
) -> Response {
    let ctx = match resolve_ctx(ext) {
        Ok(c) => c,
        Err(r) => return *r,
    };
    if let Err(r) = require_mutation_auth(&ctx, &headers).await {
        return *r;
    }
    let Ok(key) = payload.session_key.parse::<SessionKey>() else {
        return json_err(StatusCode::BAD_REQUEST, "Invalid session_key");
    };
    let agy_int = ctx.cli.sessions.interrupt(key.chat_id, key.topic_id).await;
    let cl_int = crate::cli::claude::interrupt_session(key.chat_id, key.topic_id).await;
    let cx_int = crate::cli::codex::interrupt_session(key.chat_id, key.topic_id).await;
    let mut ask_int = false;
    if let Ok(Some(sess)) = ctx.sessions.get_active(&key).await {
        for ps in sess.provider_sessions.values() {
            if !ps.session_id.is_empty() && ctx.cli.sessions.is_ask_active(&ps.session_id).await {
                ctx.cli.sessions.set_ask_active(&ps.session_id, false).await;
                ctx.events
                    .publish(WebEvent::AskResolved {
                        session_key: key.storage_key(),
                        session_id: ps.session_id.clone(),
                        summary: "Interrupted".to_string(),
                    })
                    .await;
                ask_int = true;
            }
        }
        emit_session_updated(&ctx, &sess, false, false).await;
    }
    Json(serde_json::json!({ "ok": true, "interrupted": agy_int || cl_int || cx_int || ask_int }))
        .into_response()
}

/// Executes a clean `/new` handover reset (`POST /dashboard/api/sessions/new`).
pub async fn handle_new_session(
    ext: Option<Extension<Arc<WebAppCtx>>>,
    headers: HeaderMap,
    Json(payload): Json<SessionKeyPayload>,
) -> Response {
    let ctx = match resolve_ctx(ext) {
        Ok(c) => c,
        Err(r) => return *r,
    };
    if let Err(r) = require_mutation_auth(&ctx, &headers).await {
        return *r;
    }
    let Ok(key) = payload.session_key.parse::<SessionKey>() else {
        return json_err(StatusCode::BAD_REQUEST, "Invalid session_key");
    };
    let lock = ctx.sessions.lock_pool.get((key.chat_id, key.topic_id));
    let _guard = lock.lock().await;
    let init_prov = terminate_topic_sessions_for_reset(&ctx, &key).await;
    let dm = ctx.config.model.as_deref().unwrap_or("");
    let hint = key
        .topic_id
        .and_then(|tid| ctx.topic_cache.find_by_id(key.chat_id, tid));
    let Ok(mut sess) = ctx
        .sessions
        .reset_topic_sessions(&key, &ctx.config.provider, dm, init_prov.as_deref(), hint)
        .await
    else {
        return json_err(StatusCode::INTERNAL_SERVER_ERROR, "Failed to reset session");
    };
    emit_session_updated(&ctx, &sess, true, false).await;
    let new_sid = ensure_web_session_ready_for_turn(&ctx, &mut sess, true)
        .await
        .unwrap_or_default();
    emit_session_updated(&ctx, &sess, false, false).await;
    Json(serde_json::json!({ "ok": true, "session_key": key.storage_key(), "session_id": new_sid }))
        .into_response()
}

async fn terminate_topic_sessions_for_reset(ctx: &WebAppCtx, key: &SessionKey) -> Option<String> {
    let mut init_prov = None;
    let mut sids = Vec::new();
    if let Ok(Some(ex)) = ctx.sessions.get_active(key).await {
        let ap = SessionData::effective_provider(&ex.provider, &ctx.config.provider).to_string();
        let sid = ex.get_session_id(&ap);
        if !sid.is_empty() {
            sids.push(sid);
        }
        init_prov = Some(ap);
    }
    for sid in sids {
        ctx.cli.terminate_session_clean(&sid).await;
        let _ = crate::cli::claude::terminate_topic(key.chat_id, key.topic_id, Some(&sid)).await;
        let _ = crate::cli::codex::terminate_topic(key.chat_id, key.topic_id, Some(&sid)).await;
    }
    let _ = ctx.cli.sessions.abort(key.chat_id, key.topic_id).await;
    let _ = crate::cli::claude::terminate_topic(key.chat_id, key.topic_id, None).await;
    let _ = crate::cli::codex::terminate_topic(key.chat_id, key.topic_id, None).await;
    init_prov
}

/// Deletes a Web-native (`web:0:<tid>`) topic session (`DELETE /dashboard/api/sessions`).
pub async fn handle_delete_session(
    ext: Option<Extension<Arc<WebAppCtx>>>,
    headers: HeaderMap,
    Json(payload): Json<SessionKeyPayload>,
) -> Response {
    let ctx = match resolve_ctx(ext) {
        Ok(c) => c,
        Err(r) => return *r,
    };
    if let Err(r) = require_mutation_auth(&ctx, &headers).await {
        return *r;
    }
    let Ok(key) = payload.session_key.parse::<SessionKey>() else {
        return json_err(StatusCode::BAD_REQUEST, "Invalid session_key");
    };
    if key.transport != "web" || key.chat_id != 0 {
        return json_err(StatusCode::BAD_REQUEST, "Only web-native sessions can be deleted");
    }
    let lock = ctx.sessions.lock_pool.get((key.chat_id, key.topic_id));
    let _guard = lock.lock().await;
    let _ = terminate_topic_sessions_for_reset(&ctx, &key).await;
    match ctx.sessions.delete_web_session(&key).await {
        Ok(removed) => Json(serde_json::json!({ "ok": true, "removed": removed })).into_response(),
        Err(e) => json_err(StatusCode::BAD_REQUEST, &e),
    }
}
