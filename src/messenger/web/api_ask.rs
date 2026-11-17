//! # Web Dashboard Interactive AskQuestion Handler (`api_ask.rs`)
//!
//! ## Overview
//! Handles REST submissions for active interactive `AskQuestion` cards across
//! `antigravity`, `claude`, and `codex`, synchronizing Telegram button state and
//! resuming post-answer agent output streams.
//!
//! ## Search Tags
//! #web-ask-question, #ask-resume, #ask-http

use super::api_ask_helpers::apply_ask_step_input;
use super::auth_http::{json_err, require_mutation_auth};
use super::events::{emit_session_updated, WebEvent};
use super::turn_runner::mirror_ask_message_to_telegram;
use super::turn_stream::stream_web_turn;
use super::{resolve_ctx, WebAppCtx};
use crate::cli::antigravity::session::AskState;
use crate::messenger::telegram::ask_callbacks::submit_peer_ask_answers_if_needed;
use crate::messenger::telegram::ask_helpers::build_ask_keyboard_helper;
use crate::messenger::telegram::history::log_telegram_message;
use crate::session::{data::SessionData, key::SessionKey};
use axum::{
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Extension, Json,
};
use serde::Deserialize;
use std::sync::Arc;

/// Request payload for `POST /dashboard/api/sessions/ask`.
#[derive(Deserialize)]
pub struct AskAnswerPayload {
    /// Target session key.
    pub session_key: String,
    /// Action: `answer`, `submit`, `prev`, or `skip`.
    #[serde(default = "default_ask_action")]
    pub action: String,
    /// Optional free-text or write-in answer.
    pub answer: Option<String>,
    /// Selected zero-based option indices for single or multi-select.
    #[serde(default)]
    pub selected_indices: Vec<usize>,
}

fn default_ask_action() -> String {
    "answer".to_string()
}

async fn emit_ask_step_update(
    ctx: &Arc<WebAppCtx>,
    sess: &SessionData,
    prov: &str,
    sid: &str,
    state: &AskState,
) -> Response {
    ctx.cli.sessions.set_ask_state(sid, state.clone()).await;
    let q = &state.questions[state.current_index];
    let markup = build_ask_keyboard_helper(sid, q, &state.current_bitmap, state.current_index > 0);
    mirror_ask_message_to_telegram(ctx, sess, state.msg_id, &q.question, Some(markup)).await;
    let ev = WebEvent::AskQuestion {
        session_key: sess.session_key().storage_key(),
        session_id: sid.to_string(),
        provider: prov.to_string(),
        questions: state.questions.clone(),
        current_index: state.current_index,
        answers: state.answers.clone(),
    };
    ctx.events.publish(ev.clone()).await;
    emit_session_updated(ctx, sess, false, true).await;
    Json(serde_json::json!({ "resolved": false, "ask": ev })).into_response()
}

async fn process_validated_ask(
    ctx: Arc<WebAppCtx>,
    key: SessionKey,
    payload: AskAnswerPayload,
) -> Response {
    let lock = ctx.sessions.lock_pool.get((key.chat_id, key.topic_id));
    let _guard = lock.lock().await;
    let Ok(Some(sess)) = ctx.sessions.get_active(&key).await else {
        return json_err(StatusCode::NOT_FOUND, "Session not found");
    };
    let prov = SessionData::effective_provider(&sess.provider, &ctx.config.provider);
    let sid = sess.get_session_id(prov);
    let Some(mut state) = ctx.cli.sessions.get_ask_state(&sid).await else {
        return json_err(StatusCode::NOT_FOUND, "No active AskQuestion on session");
    };
    let summary = match apply_ask_step_input(&ctx, &sess, &sid, &mut state, &payload).await {
        Ok(s) => s,
        Err(e) => return json_err(StatusCode::BAD_REQUEST, e),
    };
    log_telegram_message(&ctx.working_dir, &sid, (sess.topic_id, sess.topic_name.as_deref()), "user", None, &format!("AskQuestion ({}): {}", payload.action, summary), (true, None));
    if payload.action == "prev" {
        return emit_ask_step_update(&ctx, &sess, prov, &sid, &state).await;
    }
    if state.current_index + 1 < state.questions.len() {
        state.current_index += 1;
        let nq = &state.questions[state.current_index];
        state.current_bitmap = if nq.is_multi_select { "0".repeat(nq.options.len()) } else { String::new() };
        state.waiting_for_write_in = false;
        return emit_ask_step_update(&ctx, &sess, prov, &sid, &state).await;
    }
    finish_ask_and_resume(ctx, sess, sid, state, summary).await
}

/// Submits an answer for an active `AskQuestion` across `antigravity`, `claude`, and `codex`.
pub async fn handle_answer_ask(
    ext: Option<Extension<Arc<WebAppCtx>>>,
    headers: HeaderMap,
    Json(payload): Json<AskAnswerPayload>,
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
    process_validated_ask(ctx, key, payload).await
}

async fn mark_ask_resolved(
    ctx: &Arc<WebAppCtx>,
    sess: &SessionData,
    sid: &str,
    state: &AskState,
    summary: &str,
) {
    let sk = sess.session_key().storage_key();
    ctx.cli.sessions.set_ask_state(sid, state.clone()).await;
    ctx.cli.sessions.set_ask_active(sid, false).await;
    submit_peer_ask_answers_if_needed(sess.chat_id, sess, &ctx.config, &state.answers).await;
    if let Some(q) = state.questions.get(state.current_index) {
        let txt = format!("{}\n\n(Selected: **{}**)", q.question, summary);
        mirror_ask_message_to_telegram(ctx, sess, state.msg_id, &txt, None).await;
    }
    ctx.events
        .publish(WebEvent::AskResolved {
            session_key: sk,
            session_id: sid.to_string(),
            summary: summary.to_string(),
        })
        .await;
    emit_session_updated(ctx, sess, true, false).await;
}

async fn log_and_mirror_ask_answer(ctx: &Arc<WebAppCtx>, sess: &SessionData, sid: &str, raw_text: &str) {
    log_telegram_message(&ctx.working_dir, sid, (sess.topic_id, sess.topic_name.as_deref()), "user", None, raw_text, (true, None));
    ctx.events
        .publish(WebEvent::UserMessage {
            session_key: sess.session_key().storage_key(),
            source: "web".to_string(),
            text: raw_text.to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
        })
        .await;
    if sess.transport == "tg" {
        let _ = super::turn_runner::mirror_text_to_telegram(ctx, sess, &format!("🌐 [Web] {raw_text}")).await;
    }
}

/// Feeds a chat prompt entered during an active `AskQuestion` into the open question under topic lock.
pub(crate) async fn answer_open_ask_under_lock(
    ctx: &Arc<WebAppCtx>,
    sess: &SessionData,
    sid: &str,
    raw_text: &str,
) {
    let Some(mut state) = ctx.cli.sessions.get_ask_state(sid).await else {
        return;
    };
    let prov = SessionData::effective_provider(&sess.provider, &ctx.config.provider).to_string();
    let escaped = crate::messenger::telegram::commands_registry::escape_non_workflow_slash(raw_text.trim());
    let payload = AskAnswerPayload {
        session_key: sess.session_key().storage_key(),
        action: "answer".to_string(),
        answer: Some(escaped),
        selected_indices: Vec::new(),
    };
    let Ok(summary) = apply_ask_step_input(ctx, sess, sid, &mut state, &payload).await else {
        emit_session_updated(ctx, sess, false, true).await;
        return;
    };
    log_and_mirror_ask_answer(ctx, sess, sid, raw_text).await;
    if state.current_index + 1 < state.questions.len() {
        state.current_index += 1;
        let nq = &state.questions[state.current_index];
        state.current_bitmap = if nq.is_multi_select { "0".repeat(nq.options.len()) } else { String::new() };
        state.waiting_for_write_in = false;
        let _ = emit_ask_step_update(ctx, sess, &prov, sid, &state).await;
        return;
    }
    mark_ask_resolved(ctx, sess, sid, &state, &summary).await;
    if !cfg!(test) {
        let _fg = crate::upgrade::begin_foreground_run();
        let _tg = ctx.cli.begin_topic_turn(sess.chat_id, sess.topic_id);
        stream_web_turn(ctx, sess, sid, "").await;
    } else {
        emit_session_updated(ctx, sess, false, false).await;
    }
}

async fn finish_ask_and_resume(
    ctx: Arc<WebAppCtx>,
    sess: SessionData,
    sid: String,
    state: AskState,
    summary: String,
) -> Response {
    let prov = SessionData::effective_provider(&sess.provider, &ctx.config.provider).to_string();
    mark_ask_resolved(&ctx, &sess, &sid, &state, &summary).await;
    if !cfg!(test) {
        let turn_guard = ctx.cli.begin_topic_turn(sess.chat_id, sess.topic_id);
        let (ctx_bg, sess_bg, sid_bg) = (ctx.clone(), sess.clone(), sid);
        tokio::spawn(async move {
            let _tg = turn_guard;
            let _fg = crate::upgrade::begin_foreground_run();
            let lock = ctx_bg.sessions.lock_pool.get((sess_bg.chat_id, sess_bg.topic_id));
            let _l = lock.lock().await;
            let active = ctx_bg.sessions.get_active(&sess_bg.session_key()).await.ok().flatten().unwrap_or(sess_bg);
            if SessionData::effective_provider(&active.provider, &ctx_bg.config.provider) == prov
                && active.get_session_id(&prov) == sid_bg
            {
                stream_web_turn(&ctx_bg, &active, &sid_bg, "").await;
            }
        });
    }
    Json(serde_json::json!({ "resolved": true, "summary": summary })).into_response()
}
