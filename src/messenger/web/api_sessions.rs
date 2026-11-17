//! # Web Dashboard Session CRUD & Config Endpoints (`api_sessions.rs`)
//!
//! ## Overview
//! Lists unified `tg:*` and `web:0:*` sessions with live busy/asking state, creates
//! Web-native topic sessions, and mutates live provider/model/effort/name configuration.
//!
//! ## Search Tags
//! #web-sessions, #session-config, #topic-crud

use super::auth_http::{json_err, require_auth, require_mutation_auth};
use super::events::emit_session_updated;
use super::{resolve_ctx, WebAppCtx};
use crate::messenger::telegram::commands_model::{
    apply_model_to_active_pty, is_valid_effort_level,
};
use crate::messenger::telegram::commands_provider::switch_session_provider;
use crate::messenger::telegram::commands_registry::update_chat_commands_for_provider;
use crate::messenger::telegram::reply_directives::{
    clamp_session_effort, session_supports_effort,
};
use crate::session::{data::SessionData, key::SessionKey};
use axum::{
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Extension, Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Serialized session summary returned to the Web Dashboard sidebar and chat header.
#[derive(Clone, Debug, Serialize)]
pub struct SessionSummary {
    /// Composite storage key (`tg:<cid>[:<tid>]` or `web:0:<tid>`).
    pub session_key: String,
    /// Transport identifier (`tg` or `web`).
    pub transport: String,
    /// Telegram chat ID (`0` for Web-native topics).
    pub chat_id: i64,
    /// Forum or Web topic ID.
    pub topic_id: Option<i64>,
    /// Optional human-readable topic title.
    pub topic_name: Option<String>,
    /// Effective CLI provider (`antigravity`, `claude`, `codex`).
    pub provider: String,
    /// Effective canonical model identifier.
    pub model: String,
    /// Clamped reasoning effort level, if supported by the active model.
    pub effort: Option<String>,
    /// Active provider session ID, if initialized.
    pub session_id: String,
    /// Turn count on the active provider session.
    pub message_count: i64,
    /// Queued attachment paths waiting for the next user prompt.
    pub pending_attachments: Vec<String>,
    /// RFC3339 timestamp of most recent activity.
    pub last_active: String,
    /// True when a foreground or background turn is executing.
    pub busy: bool,
    /// True when waiting for an interactive `AskQuestion` response.
    pub asking: bool,
}

/// Checks whether `sess` currently has an in-flight turn or active `AskQuestion` state.
pub async fn check_busy_and_asking(ctx: &WebAppCtx, sess: &SessionData) -> (bool, bool) {
    let prov = SessionData::effective_provider(&sess.provider, &ctx.config.provider);
    let sid = sess.get_session_id(prov);
    let (cid, tid) = (sess.chat_id, sess.topic_id);
    let asking = !sid.is_empty()
        && ctx.cli.sessions.is_ask_active(&sid).await
        && ((cfg!(test) && sid.starts_with("mock-"))
            || ctx.cli.is_ask_backing_alive(prov, cid, tid, &sid).await);
    let busy = asking
        || ctx.cli.is_topic_turn_in_flight(cid, tid)
        || ctx.cli.sessions.boot_cancels.lock().await.contains_key(&(cid, tid))
        || (!sid.is_empty()
            && (ctx.cli.sessions.is_running(&sid).await
                || ctx.cli.sessions.is_async_running(&sid).await))
        || crate::cli::claude::is_running(cid, tid, (!sid.is_empty()).then_some(sid.as_str())).await
        || crate::cli::codex::is_running(cid, tid, (!sid.is_empty()).then_some(sid.as_str())).await;
    (busy, asking)
}

/// Builds a [`SessionSummary`] from persisted [`SessionData`] and live pool state.
pub async fn build_session_summary(ctx: &WebAppCtx, sess: &SessionData) -> SessionSummary {
    let (prov, model, effort) = SessionData::resolve_effective_target(Some(sess), &ctx.config);
    let sid = sess.get_session_id(&prov);
    let count = sess.get_message_count(&prov);
    let (busy, asking) = check_busy_and_asking(ctx, sess).await;
    SessionSummary {
        session_key: sess.session_key().storage_key(),
        transport: sess.transport.clone(),
        chat_id: sess.chat_id,
        topic_id: sess.topic_id,
        topic_name: sess.topic_name.clone(),
        provider: prov,
        model,
        effort,
        session_id: sid,
        message_count: count,
        pending_attachments: sess.pending_attachments.clone(),
        last_active: sess.last_active.clone(),
        busy,
        asking,
    }
}

/// Lists all persisted Telegram & Web sessions plus global warm pool counts.
pub async fn handle_list_sessions(
    ext: Option<Extension<Arc<WebAppCtx>>>,
    headers: HeaderMap,
) -> Response {
    let ctx = match resolve_ctx(ext) {
        Ok(c) => c,
        Err(r) => return *r,
    };
    if let Err(r) = require_auth(&ctx, &headers).await {
        return *r;
    }
    let mut all = ctx.sessions.list_all().await.unwrap_or_default();
    all.sort_by(|a, b| b.last_active.cmp(&a.last_active));
    let mut summaries = Vec::with_capacity(all.len());
    for s in &all {
        summaries.push(build_session_summary(&ctx, s).await);
    }
    let agy = ctx.cli.sessions.active_count().await;
    let claude = crate::cli::claude::active_count().await;
    let codex = crate::cli::codex::active_count().await;
    Json(serde_json::json!({
        "sessions": summaries,
        "warm_pools": { "agy": agy, "claude": claude, "codex": codex, "total_active": agy + claude + codex }
    }))
    .into_response()
}

/// Request payload for `POST /dashboard/api/sessions`.
#[derive(Deserialize)]
pub struct CreateSessionPayload {
    /// Optional topic display name.
    pub topic_name: Option<String>,
    /// Optional provider (`antigravity`, `claude`, `codex`).
    pub provider: Option<String>,
    /// Optional model identifier.
    pub model: Option<String>,
    /// Optional reasoning effort level.
    pub effort: Option<String>,
}

/// Creates a new Web-native `web:0:<tid>` topic session (`POST /dashboard/api/sessions`).
pub async fn handle_create_session(
    ext: Option<Extension<Arc<WebAppCtx>>>,
    headers: HeaderMap,
    Json(payload): Json<CreateSessionPayload>,
) -> Response {
    let ctx = match resolve_ctx(ext) {
        Ok(c) => c,
        Err(r) => return *r,
    };
    if let Err(r) = require_mutation_auth(&ctx, &headers).await {
        return *r;
    }
    let prov = payload
        .provider
        .as_deref()
        .and_then(SessionData::canonical_provider)
        .unwrap_or_else(|| SessionData::effective_provider(&ctx.config.provider, "antigravity"));
    let model = payload
        .model
        .as_deref()
        .filter(|m| !m.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| SessionData::default_model_for_provider(prov, ctx.config.model.as_deref()));
    if !SessionData::is_model_compatible(&model, prov) {
        return json_err(StatusCode::BAD_REQUEST, "Model incompatible with provider");
    }
    let name = payload.topic_name.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    match ctx.sessions.create_web_session(name, prov, &model, payload.effort.as_deref()).await {
        Ok(sess) => {
            emit_session_updated(&ctx, &sess, false, false).await;
            (StatusCode::CREATED, Json(build_session_summary(&ctx, &sess).await)).into_response()
        }
        Err(e) => json_err(StatusCode::INTERNAL_SERVER_ERROR, &e),
    }
}

/// Request payload for `PATCH /dashboard/api/sessions/config`.
#[derive(Deserialize)]
pub struct PatchConfigPayload {
    /// Target composite session key.
    pub session_key: String,
    /// Optional new topic title.
    pub topic_name: Option<String>,
    /// Optional new CLI provider.
    pub provider: Option<String>,
    /// Optional new model identifier.
    pub model: Option<String>,
    /// Optional new reasoning effort level.
    pub effort: Option<String>,
}

fn validate_model_and_effort(
    eff_prov: &str,
    model_opt: Option<&str>,
    effort_opt: Option<&str>,
) -> Result<(Option<String>, Option<String>), String> {
    let target_model = match model_opt.map(str::trim).filter(|s| !s.is_empty()) {
        Some(raw) => Some(
            SessionData::canonicalize_model_for_provider(eff_prov, raw)
                .ok_or_else(|| format!("Model `{raw}` is incompatible with `{eff_prov}`"))?,
        ),
        None => None,
    };
    let clean_eff = match effort_opt.map(str::trim).filter(|s| !s.is_empty()) {
        Some(raw) => {
            let low = raw.to_ascii_lowercase();
            if !is_valid_effort_level(&low) {
                return Err(format!("Invalid effort level `{raw}`"));
            }
            Some(low)
        }
        None => None,
    };
    Ok((target_model, clean_eff))
}

async fn apply_model_and_effort_patch(
    ctx: &WebAppCtx,
    key: &SessionKey,
    sess: &mut SessionData,
    payload: &PatchConfigPayload,
) -> Result<(), String> {
    let eff_prov = SessionData::effective_provider(&sess.provider, &ctx.config.provider).to_string();
    let (target_model, clean_eff) =
        validate_model_and_effort(&eff_prov, payload.model.as_deref(), payload.effort.as_deref())?;
    let name_opt = payload.topic_name.as_deref().map(str::trim).filter(|s| !s.is_empty());
    if let Ok(Some(updated)) = ctx
        .sessions
        .mutate_session(key, Some(sess), |cur| {
            let cp = SessionData::effective_provider(&cur.provider, &ctx.config.provider).to_string();
            cur.provider = cp.clone();
            if let Some(ref m) = target_model {
                cur.model = m.clone();
            }
            if let Some(ref eff) = clean_eff {
                cur.effort = clamp_session_effort(&cp, &cur.model, eff);
            } else if target_model.is_some() && !session_supports_effort(&cp, &cur.model) {
                cur.effort = None;
            }
            if let Some(tname) = name_opt {
                cur.topic_name = Some(tname.to_string());
            }
            true
        })
        .await
    {
        *sess = updated;
    }
    apply_model_to_active_pty(&ctx.cli, &ctx.sessions, sess).await;
    Ok(())
}

/// Updates session provider, model, effort, or topic name (`PATCH /dashboard/api/sessions/config`).
pub async fn handle_patch_session_config(
    ext: Option<Extension<Arc<WebAppCtx>>>,
    headers: HeaderMap,
    Json(payload): Json<PatchConfigPayload>,
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
    let Ok(Some(mut sess)) = ctx.sessions.get_active(&key).await else {
        return json_err(StatusCode::NOT_FOUND, "Session not found");
    };
    if let Some(ref raw_p) = payload.provider {
        let Some(target_p) = SessionData::canonical_provider(raw_p) else {
            return json_err(StatusCode::BAD_REQUEST, "Unknown provider");
        };
        let cur_p = SessionData::effective_provider(&sess.provider, &ctx.config.provider);
        if cur_p != target_p {
            let _ = switch_session_provider(target_p, &ctx.config, &ctx.sessions, &ctx.cli, &mut sess).await;
            if key.transport == "tg" && let Some(bot) = ctx.bot_clone().await {
                update_chat_commands_for_provider(&bot, teloxide::types::ChatId(key.chat_id), target_p).await;
            }
        }
    }
    if let Err(msg) = apply_model_and_effort_patch(&ctx, &key, &mut sess, &payload).await {
        return json_err(StatusCode::BAD_REQUEST, &msg);
    }
    let (busy, asking) = check_busy_and_asking(&ctx, &sess).await;
    emit_session_updated(&ctx, &sess, busy, asking).await;
    Json(build_session_summary(&ctx, &sess).await).into_response()
}
