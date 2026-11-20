//! # Web Dashboard History & File Upload Endpoints (`api_history.rs`)
//!
//! ## Overview
//! Reads merged `telegram_history.jsonl` transcripts, active `AskQuestion` cards, and live
//! streaming buffers for any session key, and handles multipart file uploads into
//! `telegram_files/` with atomic pending attachment registration.
//!
//! ## Search Tags
//! #chat-history, #multipart-upload, #active-ask, #live-buffer

use super::api_files::filter_safe_preview_paths;
use super::api_sessions::{build_session_summary, check_busy_and_asking};
use super::auth_http::{json_err, require_auth, require_mutation_auth};
use super::events::emit_session_updated;
use super::{resolve_ctx, WebAppCtx};
use crate::cli::AskQuestionData;
use crate::messenger::telegram::handover_helpers::extract_topic_history;
use crate::messenger::telegram::media::sanitize_filename;
use crate::session::{data::SessionData, key::SessionKey};
use axum::{
    extract::{Multipart, Query},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Extension, Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Active `AskQuestion` snapshot returned with session history.
#[derive(Clone, Debug, Serialize)]
pub struct ActiveAskView {
    /// Composite session key.
    pub session_key: String,
    /// Provider session ID.
    pub session_id: String,
    /// Active CLI provider name.
    pub provider: String,
    /// Ordered list of questions.
    pub questions: Vec<AskQuestionData>,
    /// Zero-based index of the current question.
    pub current_index: usize,
    /// Answers recorded for prior steps.
    pub answers: Vec<String>,
}

/// Query parameters for `GET /dashboard/api/sessions/history`.
#[derive(Deserialize)]
pub struct HistoryQuery {
    /// Target composite session key.
    pub session_key: String,
}

/// Builds an [`ActiveAskView`] snapshot when `sess` has a live `AskQuestion` prompt.
pub(crate) async fn build_active_ask_view(
    ctx: &WebAppCtx,
    sess: &SessionData,
    prov: &str,
    sid: &str,
) -> Option<ActiveAskView> {
    let alive = (cfg!(test) && sid.starts_with("mock-"))
        || ctx
            .cli
            .is_ask_backing_alive(prov, sess.chat_id, sess.topic_id, sid)
            .await;
    if sid.is_empty() || !ctx.cli.sessions.is_ask_active(sid).await || !alive {
        return None;
    }
    let st = ctx.cli.sessions.get_ask_state(sid).await?;
    Some(ActiveAskView {
        session_key: sess.session_key().storage_key(),
        session_id: sid.to_string(),
        provider: prov.to_string(),
        questions: st.questions,
        current_index: st.current_index,
        answers: st.answers,
    })
}

pub(crate) fn collect_history_messages(
    ctx: &WebAppCtx,
    sess: &SessionData,
    prov: &str,
    sid: &str,
) -> Vec<serde_json::Value> {
    let mut sids = sess.get_past_session_ids(prov);
    if !sid.is_empty() && !sids.contains(&sid.to_string()) {
        sids.push(sid.to_string());
    }
    let mut messages = Vec::new();
    for id in &sids {
        for entry in extract_topic_history(&ctx.working_dir, id) {
            let files = filter_safe_preview_paths(ctx, &entry.text);
            messages.push(serde_json::json!({
                "session_id": id,
                "timestamp": entry.timestamp,
                "sender": entry.sender,
                "text": entry.text,
                "is_success": entry.is_success,
                "error": entry.error,
                "files": files
            }));
        }
    }
    messages
}

/// Returns message history, active `AskQuestion`, and live streaming buffer for a session.
pub async fn handle_session_history(
    ext: Option<Extension<Arc<WebAppCtx>>>,
    headers: HeaderMap,
    Query(q): Query<HistoryQuery>,
) -> Response {
    let ctx = match resolve_ctx(ext) {
        Ok(c) => c,
        Err(r) => return *r,
    };
    if let Err(r) = require_auth(&ctx, &headers).await {
        return *r;
    }
    let Ok(key) = q.session_key.parse::<SessionKey>() else {
        return json_err(StatusCode::BAD_REQUEST, "Invalid session_key");
    };
    let Ok(Some(sess)) = ctx.sessions.get_active(&key).await else {
        return json_err(StatusCode::NOT_FOUND, "Session not found");
    };
    let prov = SessionData::effective_provider(&sess.provider, &ctx.config.provider);
    let sid = sess.get_session_id(prov);
    let messages = collect_history_messages(&ctx, &sess, prov, &sid);
    let active_ask = build_active_ask_view(&ctx, &sess, prov, &sid).await;
    let live_stream = ctx.events.get_live_stream(&q.session_key).await;
    let summary = build_session_summary(&ctx, &sess).await;
    Json(serde_json::json!({
        "session": summary,
        "messages": messages,
        "active_ask": active_ask,
        "live_stream": live_stream
    }))
    .into_response()
}

async fn read_multipart_upload(
    mut multipart: Multipart,
) -> (Option<String>, Option<(String, Vec<u8>)>) {
    let mut session_key = None;
    let mut first_file = None;
    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.name().unwrap_or("").to_string();
        if name == "session_key" {
            if let Ok(t) = field.text().await {
                session_key = Some(t.trim().to_string());
            }
        } else if name == "file" && first_file.is_none() {
            let fname = field.file_name().unwrap_or("upload.bin").to_string();
            if let Ok(bytes) = field.bytes().await {
                first_file = Some((fname, bytes.to_vec()));
            }
        }
    }
    (session_key, first_file)
}

/// Handles multipart file uploads (`POST /dashboard/api/sessions/upload`) into `telegram_files/`.
pub async fn handle_upload_file(
    ext: Option<Extension<Arc<WebAppCtx>>>,
    headers: HeaderMap,
    multipart: Multipart,
) -> Response {
    let ctx = match resolve_ctx(ext) {
        Ok(c) => c,
        Err(r) => return *r,
    };
    if let Err(r) = require_mutation_auth(&ctx, &headers).await {
        return *r;
    }
    let (session_key, first_file) = read_multipart_upload(multipart).await;
    let Some((raw_name, bytes)) = first_file else {
        return json_err(StatusCode::BAD_REQUEST, "Missing file field");
    };
    let clean = sanitize_filename(&raw_name);
    let safe_name = if clean.is_empty() { "upload.bin".to_string() } else { clean };
    let stored_name = format!("web_{}_{}", chrono::Utc::now().timestamp_millis(), safe_name);
    let dest_dir = ctx.working_dir.join("telegram_files");
    if tokio::fs::create_dir_all(&dest_dir).await.is_err() {
        return json_err(StatusCode::INTERNAL_SERVER_ERROR, "Failed to create upload dir");
    }
    if tokio::fs::write(dest_dir.join(&stored_name), &bytes).await.is_err() {
        return json_err(StatusCode::INTERNAL_SERVER_ERROR, "Failed to write uploaded file");
    }
    let rel_path = format!("telegram_files/{stored_name}");
    let pending = attach_uploaded_file(&ctx, session_key.as_deref(), &rel_path).await;
    Json(serde_json::json!({
        "path": rel_path,
        "filename": stored_name,
        "size_bytes": bytes.len(),
        "pending_attachments": pending
    }))
    .into_response()
}

async fn attach_uploaded_file(
    ctx: &Arc<WebAppCtx>,
    sk_opt: Option<&str>,
    rel_path: &str,
) -> Vec<String> {
    let Some(sk) = sk_opt.and_then(|s| s.parse::<SessionKey>().ok()) else {
        return vec![rel_path.to_string()];
    };
    let Ok(Some(updated)) = ctx
        .sessions
        .mutate_session(&sk, None, |cur| {
            if !cur.pending_attachments.contains(&rel_path.to_string()) {
                cur.pending_attachments.push(rel_path.to_string());
                return true;
            }
            false
        })
        .await
    else {
        return vec![rel_path.to_string()];
    };
    let (busy, asking) = check_busy_and_asking(ctx, &updated).await;
    emit_session_updated(ctx, &updated, busy, asking).await;
    updated.pending_attachments
}
