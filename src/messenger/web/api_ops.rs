//! # Operational Usage, Cron & HITL Control API (`api_ops.rs`)
//!
//! ## Overview
//! Exposes structured multi-provider `/usage` telemetry (`antigravity`, `claude`, `codex`),
//! `/cron` schedule inspection, toggle, and run-now endpoints, plus HITL auth session status.
//!
//! ## Search Tags
//! #web-usage, #web-cron, #hitl-status

use super::auth_http::{json_err, require_auth, require_mutation_auth};
use super::events::WebEvent;
use super::{resolve_ctx, WebAppCtx};
use crate::cron::{calculate_job_next_run, CronScheduler};
use crate::messenger::telegram::commands_usage_agy::query_antigravity_quota_with_fallback;
use crate::messenger::telegram::commands_usage_claude::{
    fetch_claude_oauth_quota, read_claude_session_stats,
};
use crate::messenger::telegram::commands_usage_codex::{
    query_codex_app_server_rate_limits, read_codex_session_telemetry,
};
use crate::session::{data::SessionData, key::SessionKey};
use axum::{
    extract::{Path as AxumPath, Query},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Extension, Json,
};
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::Arc;

/// Query parameters for `GET /dashboard/api/usage`.
#[derive(Deserialize)]
pub struct UsageQuery {
    /// Optional target session key for per-session Claude/Codex token stats.
    #[serde(default)]
    pub session_key: String,
    /// When `1`, fetches all 3 providers concurrently.
    #[serde(default)]
    pub all: u8,
}

async fn query_claude_json(sess: &SessionData) -> serde_json::Value {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    let creds = PathBuf::from(&home).join(".claude/.credentials.json");
    let base = std::env::var("ANTHROPIC_BASE_URL").unwrap_or_else(|_| "https://api.anthropic.com".to_string());
    let (sub_type, quota) = fetch_claude_oauth_quota(&creds, &base).await;
    let sid = sess.get_session_id("claude");
    let (inp, outp, cread, cwrite, turns) = read_claude_session_stats(&sid);
    let auth_label = sub_type.map(|s| format!("Claude OAuth ({s})")).unwrap_or_else(|| "API Key / Local".to_string());
    serde_json::json!({
        "auth_label": auth_label,
        "quota": quota,
        "turns": turns,
        "input_tokens": inp,
        "output_tokens": outp,
        "cache_read": cread,
        "cache_write": cwrite
    })
}

async fn query_codex_json(sess: &SessionData) -> serde_json::Value {
    let (mut acc, mut grp) = query_codex_app_server_rate_limits().await;
    let sid = sess.get_session_id("codex");
    let (inp, cached, outp, reas, total, ctx_win, fb_plan, fb_grp) = read_codex_session_telemetry(&sid).await;
    if acc.is_none() { acc = fb_plan; }
    if grp.is_none() { grp = fb_grp; }
    serde_json::json!({
        "auth_label": acc.unwrap_or_else(|| "API Key / Custom".to_string()),
        "quota": grp,
        "input_tokens": inp,
        "cached_input": cached,
        "output_tokens": outp,
        "reasoning_tokens": reas,
        "total_tokens": total,
        "context_window": ctx_win
    })
}

/// Returns structured multi-provider quota & token telemetry (`GET /dashboard/api/usage`).
pub async fn handle_get_usage(
    ext: Option<Extension<Arc<WebAppCtx>>>,
    headers: HeaderMap,
    Query(q): Query<UsageQuery>,
) -> Response {
    let ctx = match resolve_ctx(ext) {
        Ok(c) => c,
        Err(r) => return *r,
    };
    if let Err(r) = require_auth(&ctx, &headers).await {
        return *r;
    }
    let sess = if let Ok(key) = q.session_key.parse::<SessionKey>() {
        ctx.sessions.get_active(&key).await.ok().flatten().unwrap_or_default()
    } else {
        SessionData::default()
    };
    let active_prov = SessionData::effective_provider(&sess.provider, &ctx.config.provider);
    let fetch_all = q.all == 1;
    let (agy_res, cl_val, cx_val) = tokio::join!(
        async {
            if fetch_all || active_prov == "antigravity" {
                Some(query_antigravity_quota_with_fallback(&ctx.sessions, &ctx.cli, &sess, sess.chat_id, sess.topic_id).await)
            } else {
                None
            }
        },
        async { if fetch_all || active_prov == "claude" { Some(query_claude_json(&sess).await) } else { None } },
        async { if fetch_all || active_prov == "codex" { Some(query_codex_json(&sess).await) } else { None } }
    );
    let agy_val = agy_res.map(|res| match res {
        Ok(rep) => serde_json::json!({ "account": rep.account, "gemini": rep.gemini, "claude": rep.claude }),
        Err(e) => serde_json::json!({ "error": e }),
    });
    Json(serde_json::json!({
        "active_provider": active_prov,
        "antigravity": agy_val,
        "claude": cl_val,
        "codex": cx_val
    }))
    .into_response()
}

/// Lists all cron jobs with computed next local run times (`GET /dashboard/api/cron`).
pub async fn handle_list_cron(
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
    let jobs = ctx.cron_manager.list_jobs().await.unwrap_or_default();
    let default_tz = ctx.config.user_timezone.as_deref().unwrap_or("UTC");
    let list: Vec<serde_json::Value> = jobs
        .into_iter()
        .map(|j| {
            let eff_tz = if j.timezone.is_empty() { default_tz } else { &j.timezone };
            let next_run = calculate_job_next_run(&j.schedule, eff_tz)
                .ok()
                .map(|dt| dt.format("%Y-%m-%d %H:%M %Z").to_string());
            serde_json::json!({
                "id": j.id,
                "title": j.title,
                "schedule": j.schedule,
                "timezone": eff_tz,
                "enabled": j.enabled,
                "last_run_at": j.last_run_at,
                "last_run_status": j.last_run_status,
                "next_run": next_run,
                "provider": j.provider,
                "model": j.model
            })
        })
        .collect();
    Json(serde_json::json!({ "jobs": list })).into_response()
}

/// Toggles a cron job's `enabled` state (`POST /dashboard/api/cron/:id/toggle`).
pub async fn handle_toggle_cron(
    ext: Option<Extension<Arc<WebAppCtx>>>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
) -> Response {
    let ctx = match resolve_ctx(ext) {
        Ok(c) => c,
        Err(r) => return *r,
    };
    if let Err(r) = require_mutation_auth(&ctx, &headers).await {
        return *r;
    }
    let Ok(Some(job)) = ctx.cron_manager.get_job(&id).await else {
        return json_err(StatusCode::NOT_FOUND, "Cron job not found");
    };
    let next_enabled = !job.enabled;
    if let Err(e) = ctx.cron_manager.set_enabled(&id, next_enabled).await {
        return json_err(StatusCode::INTERNAL_SERVER_ERROR, &e);
    }
    ctx.events
        .publish(WebEvent::CronUpdated {
            job_id: id.clone(),
            enabled: next_enabled,
            last_run_status: job.last_run_status.clone(),
        })
        .await;
    Json(serde_json::json!({ "ok": true, "id": id, "enabled": next_enabled })).into_response()
}

/// Optional request body for `POST /dashboard/api/cron/:id/run`.
#[derive(Default, Deserialize)]
pub struct RunCronPayload {
    /// Active session key on the caller's Dashboard tab (e.g. `web:0:1` or `tg:...`).
    #[serde(default)]
    pub session_key: Option<String>,
}

async fn resolve_cron_exec_target(
    ctx: &WebAppCtx,
    job: &crate::cron::manager::CronJob,
    session_key: Option<&str>,
) -> (Arc<crate::config::CliConfig>, i64, Option<i64>) {
    let has_bot = ctx.bot.read().await.is_some();
    let parsed_sk = session_key.and_then(|s| s.trim().parse::<SessionKey>().ok());
    if !has_bot
        || (job.chat_id == 0 && parsed_sk.as_ref().is_some_and(|k| k.transport == "web"))
    {
        let mut web_cfg = (*ctx.config).clone();
        web_cfg.transport = "web".to_string();
        let tid = parsed_sk
            .as_ref()
            .filter(|k| k.transport == "web")
            .and_then(|k| k.topic_id)
            .or(Some(1));
        return (Arc::new(web_cfg), 0, tid);
    }
    if let Some(ref k) = parsed_sk
        && k.transport == "tg"
        && k.chat_id != 0
    {
        return (ctx.config.clone(), k.chat_id, k.topic_id);
    }
    let fallback_cid = ctx
        .config
        .allowed_group_ids
        .first()
        .or_else(|| ctx.config.allowed_user_ids.first())
        .copied()
        .unwrap_or(0);
    if fallback_cid == 0 && job.chat_id == 0 {
        let mut web_cfg = (*ctx.config).clone();
        web_cfg.transport = "web".to_string();
        return (Arc::new(web_cfg), 0, Some(1));
    }
    (ctx.config.clone(), fallback_cid, None)
}

/// Triggers immediate cron job execution (`POST /dashboard/api/cron/:id/run`).
pub async fn handle_run_cron(
    ext: Option<Extension<Arc<WebAppCtx>>>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
    body: Option<Json<RunCronPayload>>,
) -> Response {
    let ctx = match resolve_ctx(ext) {
        Ok(c) => c,
        Err(r) => return *r,
    };
    if let Err(r) = require_mutation_auth(&ctx, &headers).await {
        return *r;
    }
    let Ok(Some(job)) = ctx.cron_manager.get_job(&id).await else {
        return json_err(StatusCode::NOT_FOUND, "Cron job not found");
    };
    let req_sk = body.as_ref().and_then(|b| b.session_key.as_deref());
    let (sched_cfg, caller_cid, caller_tid) = resolve_cron_exec_target(&ctx, &job, req_sk).await;
    let scheduler = CronScheduler::new(sched_cfg, ctx.cron_manager.clone(), ctx.bus.clone());
    let job_id = job.id.clone();
    let enabled = job.enabled;
    let events = ctx.events.clone();
    let mgr = ctx.cron_manager.clone();
    tokio::spawn(async move {
        let _ = scheduler.execute_job_now(job, caller_cid, caller_tid).await;
        let status = mgr.get_job(&job_id).await.ok().flatten().and_then(|j| j.last_run_status);
        events
            .publish(WebEvent::CronUpdated { job_id, enabled, last_run_status: status })
            .await;
    });
    (StatusCode::ACCEPTED, Json(serde_json::json!({ "ok": true, "id": id }))).into_response()
}

/// Reports whether any HITL remote browser auth session is active (`GET /dashboard/api/hitl`).
pub async fn handle_hitl_status(
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
    let active = crate::security::vault_auth::proxy::has_active_auth_sessions().await;
    Json(serde_json::json!({ "active": active })).into_response()
}
