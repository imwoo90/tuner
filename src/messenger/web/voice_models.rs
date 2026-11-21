//! # Voice Secretary Model Alias Resolver & Catalog Tools (`voice_models.rs`)
//!
//! ## Overview
//! Normalizes spoken Korean/English provider, model, and reasoning-effort names
//! (e.g. `"gemini 4 hydrogen"` or phonetic aliases -> `"gemini-4-hydrogen"`),
//! lists live discovered models, and reconfigures existing topics by voice.
//!
//! ## Search Tags
//! #voice-models, #spoken-model-alias, #topic-config, #model-discovery

use super::api_sessions::{build_session_summary, check_busy_and_asking};
use super::events::emit_session_updated;
use super::WebAppCtx;
use crate::messenger::telegram::callbacks_effort::{
    discover_models_with_default, group_discovered_models,
};
use crate::messenger::telegram::commands_model::{apply_model_to_active_pty, is_valid_effort_level};
use crate::messenger::telegram::commands_provider::switch_session_provider;
use crate::messenger::telegram::reply_directives::{clamp_session_effort, session_supports_effort};
use crate::session::{data::SessionData, key::SessionKey};
use serde_json::{json, Value};
use std::sync::Arc;

fn canonicalize_spoken_provider(raw: &str) -> Option<&'static str> {
    let clean = raw.trim().to_ascii_lowercase();
    if let Some(p) = SessionData::canonical_provider(&clean) {
        return Some(p);
    }
    if clean.contains("클로드") || clean.contains("앤트로픽") || clean.contains("claude") {
        Some("claude")
    } else if clean.contains("코덱스") || clean.contains("오픈ai") || clean.contains("codex") || clean == "openai" {
        Some("codex")
    } else if clean.contains("안티그래비티") || clean.contains("제미나이") || clean.contains("antigravity") {
        Some("antigravity")
    } else {
        None
    }
}

fn map_spoken_model_token(raw: &str) -> String {
    let mut s = raw.trim().to_ascii_lowercase().replace(['_', ' '], "-");
    while s.contains("--") {
        s = s.replace("--", "-");
    }
    if s.contains("하이드로젠") || s.contains("hydrogen") {
        return "gemini-4-hydrogen".to_string();
    }
    if s.contains("네온") || s.contains("neon") {
        return "gemini-4-neon".to_string();
    }
    if s.contains("오케스트레이터") || s.contains("orchestrator") {
        return if s.contains("아르곤") || s.contains("argon") {
            "gemini-pro-orchestrator-argon".to_string()
        } else {
            "gemini-pro-orchestrator".to_string()
        };
    }
    if s.contains("아르곤") || s.contains("argon") {
        return "gemini-4-argon".to_string();
    }
    if s.contains("카본") || s.contains("carbon") {
        return "gemini-4-carbon".to_string();
    }
    if s.contains("3.8") || (s.contains("플래시") && s.contains("3")) {
        return "gemini-3.8-flash".to_string();
    }
    if s.contains("소넷") || s == "sonnet" {
        return "sonnet".to_string();
    }
    if s.contains("오퍼스") || s == "opus" {
        return "opus".to_string();
    }
    if s.contains("하이쿠") || s == "haiku" {
        return "haiku".to_string();
    }
    if s.contains("아스트라") || s == "astra" {
        return "astra".to_string();
    }
    if s.contains("루나") || s == "luna" {
        return "luna".to_string();
    }
    s.replace("gemini4-", "gemini-4-").replace("gemini3.", "gemini-3.")
}

fn infer_provider_for_model(base_prov: &str, slug: &str) -> &'static str {
    let cur = SessionData::canonical_provider(base_prov).unwrap_or("antigravity");
    if SessionData::canonicalize_model_for_provider(cur, slug).is_some() {
        return cur;
    }
    for cand in ["antigravity", "claude", "codex"] {
        if SessionData::canonicalize_model_for_provider(cand, slug).is_some() {
            return cand;
        }
    }
    cur
}

/// Resolves optional spoken `(provider, model, effort)` inputs into validated canonical values.
pub async fn resolve_spoken_model_spec(
    ctx: &WebAppCtx,
    fallback_prov: &str,
    raw_prov: Option<&str>,
    raw_model: Option<&str>,
    raw_effort: Option<&str>,
) -> Result<(String, String, Option<String>), String> {
    crate::cli::antigravity::model_spec::ensure_live_catalog_fresh().await;
    let explicit_prov = match raw_prov.map(str::trim).filter(|s| !s.is_empty()) {
        Some(rp) => Some(
            canonicalize_spoken_provider(rp)
                .ok_or_else(|| format!("Unknown provider `{rp}` (use antigravity, claude, or codex)"))?,
        ),
        None => None,
    };
    let model_slug = raw_model
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(map_spoken_model_token);

    let prov = match (explicit_prov, model_slug.as_deref()) {
        (Some(p), _) => p,
        (None, Some(m)) => infer_provider_for_model(fallback_prov, m),
        (None, None) => SessionData::effective_provider(fallback_prov, &ctx.config.provider),
    };

    let model = match model_slug.as_deref() {
        Some(m) => SessionData::canonicalize_model_for_provider(prov, m)
            .ok_or_else(|| format!("Model `{m}` is not supported on provider `{prov}`"))?,
        None => SessionData::default_model_for_provider(prov, ctx.config.model.as_deref()),
    };

    let eff_seed = raw_effort
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .or(ctx.config.effort.as_deref());
    let effort = match eff_seed {
        Some(e) => {
            let low = e.to_ascii_lowercase();
            if raw_effort.is_some() && !is_valid_effort_level(&low) {
                return Err(format!("Invalid effort level `{e}`"));
            }
            clamp_session_effort(prov, &model, &low)
        }
        None => None,
    };
    Ok((prov.to_string(), model, effort))
}

async fn persist_voice_spec_mutation(
    ctx: &Arc<WebAppCtx>,
    key: &SessionKey,
    sess: &mut SessionData,
    cur_p: &str,
    spec: (&str, &str, Option<&str>),
    has_raw_eff_or_mod: bool,
    clean_title: Option<&str>,
) {
    let (target_p, target_m, target_e) = spec;
    if let Ok(Some(updated)) = ctx
        .sessions
        .mutate_session(key, Some(sess), |cur| {
            cur.provider = target_p.to_string();
            cur.model = target_m.to_string();
            if has_raw_eff_or_mod || target_p != cur_p {
                cur.effort = if session_supports_effort(target_p, target_m) {
                    target_e.map(str::to_string).or_else(|| {
                        cur.effort.as_deref().and_then(|prev| clamp_session_effort(target_p, target_m, prev))
                    })
                } else {
                    None
                };
            }
            if let Some(t) = clean_title {
                cur.topic_name = Some(t.to_string());
            }
            true
        })
        .await
    {
        *sess = updated;
    }
    apply_model_to_active_pty(&ctx.cli, &ctx.sessions, sess).await;
    if let Some(t) = clean_title {
        super::tg_topics::sync_remote_topic_rename(ctx, key, t).await;
    }
    let (busy, asking) = check_busy_and_asking(ctx, sess).await;
    emit_session_updated(ctx, sess, busy, asking).await;
}

/// Applies an optional spoken `(provider, model, effort, topic_name)` update to an existing topic.
pub async fn apply_voice_spec_to_session(
    ctx: &Arc<WebAppCtx>,
    key: &SessionKey,
    raw_prov: Option<&str>,
    raw_model: Option<&str>,
    raw_effort: Option<&str>,
    new_title: Option<&str>,
) -> Result<(String, String, Option<String>), String> {
    let mut sess = ctx
        .sessions
        .get_active(key)
        .await
        .ok()
        .flatten()
        .ok_or_else(|| "Topic not found".to_string())?;
    let cur_p = SessionData::effective_provider(&sess.provider, &ctx.config.provider).to_string();
    let has_spec = raw_prov.is_some() || raw_model.is_some() || raw_effort.is_some();
    if !has_spec && new_title.is_none() {
        return Ok(SessionData::resolve_effective_target(Some(&sess), &ctx.config));
    }
    let (target_p, target_m, target_e) = if has_spec {
        let seed_m = raw_model.or(if raw_prov.is_none() { Some(sess.model.as_str()) } else { None });
        resolve_spoken_model_spec(ctx, &cur_p, raw_prov, seed_m, raw_effort).await?
    } else {
        SessionData::resolve_effective_target(Some(&sess), &ctx.config)
    };
    if target_p != cur_p {
        let _ = switch_session_provider(&target_p, &ctx.config, &ctx.sessions, &ctx.cli, &mut sess).await;
    }
    let clean_title = new_title.map(str::trim).filter(|s| !s.is_empty());
    persist_voice_spec_mutation(
        ctx,
        key,
        &mut sess,
        &cur_p,
        (&target_p, &target_m, target_e.as_deref()),
        raw_effort.is_some() || raw_model.is_some(),
        clean_title,
    )
    .await;
    let (_, _, final_eff) = SessionData::resolve_effective_target(Some(&sess), &ctx.config);
    Ok((target_p, target_m, final_eff))
}

/// Lists discovered models and supported effort levels (`list_available_models`).
pub async fn exec_list_available_models(ctx: &WebAppCtx, args: &Value) -> Value {
    let filter_p = args
        .get("provider")
        .and_then(Value::as_str)
        .and_then(canonicalize_spoken_provider);
    let providers: Vec<&str> = match filter_p {
        Some(p) => vec![p],
        None => vec!["antigravity", "claude", "codex"],
    };
    let mut catalog = serde_json::Map::new();
    for prov in providers {
        let raw = discover_models_with_default(&ctx.cli, prov).await;
        let models: Vec<Value> = group_discovered_models(&raw)
            .into_iter()
            .map(|(m, effs)| json!({ "model": m, "efforts": effs }))
            .collect();
        catalog.insert(prov.to_string(), Value::Array(models));
    }
    let (_, def_m, def_e) = SessionData::resolve_effective_target(None, &ctx.config);
    json!({ "default_model": def_m, "default_effort": def_e, "providers": catalog })
}

/// Executes `configure_topic` tool (`provider` / `model` / `effort` / `topic_name` mutation).
pub async fn exec_configure_topic(ctx: &Arc<WebAppCtx>, key: &SessionKey, args: &Value) -> Value {
    let rp = args.get("provider").and_then(Value::as_str);
    let rm = args.get("model").and_then(Value::as_str);
    let re = args.get("effort").and_then(Value::as_str);
    let rt = args.get("topic_name").and_then(Value::as_str);
    match apply_voice_spec_to_session(ctx, key, rp, rm, re, rt).await {
        Ok(_) => match ctx.sessions.get_active(key).await.ok().flatten() {
            Some(s) => json!({ "ok": true, "topic": build_session_summary(ctx, &s).await }),
            None => json!({ "ok": true, "session_key": key.storage_key() }),
        },
        Err(e) => json!({ "error": e }),
    }
}
