//! # Telegram Forum & Bot-DM Topic Lifecycle Bridge (`tg_topics.rs`)
//!
//! ## Overview
//! Creates, renames, and deletes real Telegram forum / `has_topics_enabled` private-chat
//! topics via the Telegram Bot API (`createForumTopic`, `editForumTopic`, `deleteForumTopic`)
//! so topics initiated from the Web Dashboard's `Telegram` tab sync 1:1 with Telegram apps.
//!
//! ## Search Tags
//! #telegram-topics, #create-forum-topic, #web-telegram-sync

use super::api_sessions::CreateSessionPayload;
use super::auth_http::json_err;
use super::WebAppCtx;
use crate::session::{data::SessionData, key::SessionKey};
use axum::{http::StatusCode, response::Response};
use serde::Deserialize;
use std::time::Duration;
use teloxide::Bot;

#[derive(Deserialize)]
struct TgApiEnvelope<T> {
    ok: bool,
    result: Option<T>,
    description: Option<String>,
}

#[derive(Deserialize)]
struct CreatedTopicResult {
    message_thread_id: i64,
    name: Option<String>,
}

fn clamp_topic_title(raw: Option<&str>) -> String {
    let trimmed = raw.map(str::trim).filter(|s| !s.is_empty());
    let base = match trimmed {
        Some(s) => s.to_string(),
        None => format!("Tuner · {}", chrono::Local::now().format("%m/%d %H:%M")),
    };
    base.chars().take(120).collect()
}

fn bot_method_url(bot: &Bot, method: &str) -> String {
    let url = bot.api_url();
    let base = url.as_str().trim_end_matches('/');
    format!("{base}/bot{}/{method}", bot.token())
}

async fn collect_candidate_chat_ids(ctx: &WebAppCtx, preferred: Option<i64>) -> Vec<i64> {
    let mut out = Vec::new();
    let mut push_unique = |cid: i64| {
        if cid != 0 && !out.contains(&cid) {
            out.push(cid);
        }
    };

    let mut all = ctx.sessions.list_all().await.unwrap_or_default();
    all.sort_by(|a, b| b.last_active.cmp(&a.last_active));

    if let Some(pref) = preferred.filter(|&c| c != 0) {
        let allowed = ctx.config.allowed_group_ids.contains(&pref)
            || ctx.config.allowed_user_ids.contains(&pref)
            || all.iter().any(|s| s.transport == "tg" && s.chat_id == pref);
        if allowed {
            push_unique(pref);
        }
    }

    for s in &all {
        if s.transport == "tg" && s.topic_id.is_some() {
            push_unique(s.chat_id);
        }
    }
    for &gid in &ctx.config.allowed_group_ids {
        push_unique(gid);
    }
    for &uid in &ctx.config.allowed_user_ids {
        push_unique(uid);
    }
    out
}

async fn call_create_forum_topic(
    client: &reqwest::Client,
    bot: &Bot,
    chat_id: i64,
    title: &str,
) -> Result<(i64, String), String> {
    let resp = client
        .post(bot_method_url(bot, "createForumTopic"))
        .json(&serde_json::json!({ "chat_id": chat_id, "name": title }))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let parsed: TgApiEnvelope<CreatedTopicResult> =
        resp.json().await.map_err(|e| e.to_string())?;
    if !parsed.ok {
        return Err(parsed.description.unwrap_or_else(|| "Telegram rejected createForumTopic".into()));
    }
    let Some(res) = parsed.result else {
        return Err("Missing result from createForumTopic".into());
    };
    let final_name = res.name.filter(|s| !s.trim().is_empty()).unwrap_or_else(|| title.to_string());
    Ok((res.message_thread_id, final_name))
}

fn spawn_starter_notice(bot: Bot, chat_id: i64, thread_id: i64, prov: String, model: String) {
    tokio::spawn(async move {
        let Ok(client) = reqwest::Client::builder().timeout(Duration::from_secs(8)).build() else {
            return;
        };
        let text = format!("🖥️ [Web Dashboard] 새 토픽이 생성되었습니다 ({prov} · {model})");
        let _ = client
            .post(bot_method_url(&bot, "sendMessage"))
            .json(&serde_json::json!({
                "chat_id": chat_id,
                "message_thread_id": thread_id,
                "text": text,
                "disable_notification": true
            }))
            .send()
            .await;
    });
}

/// Creates a real Telegram topic via `createForumTopic` and persists `tg:<cid>:<tid>`.
pub async fn create_remote_telegram_topic(
    ctx: &WebAppCtx,
    raw_title: Option<&str>,
    preferred_chat_id: Option<i64>,
    prov: &str,
    model: &str,
    effort: Option<&str>,
    send_starter: bool,
) -> Result<SessionData, (StatusCode, String)> {
    let Some(bot) = ctx.bot_clone().await else {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "Telegram Bot is not connected (Web-Standalone mode)".into(),
        ));
    };
    let candidates = collect_candidate_chat_ids(ctx, preferred_chat_id).await;
    if candidates.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "No authorized Telegram chat_id configured yet. Send a message in Telegram first.".into(),
        ));
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let title = clamp_topic_title(raw_title);
    let mut last_err = String::from("Failed to create Telegram topic");
    for cid in candidates {
        match call_create_forum_topic(&client, &bot, cid, &title).await {
            Ok((tid, final_name)) => {
                ctx.topic_cache.insert(cid, tid, final_name.clone());
                let sess = ctx
                    .sessions
                    .create_telegram_topic_session(cid, tid, final_name, prov, model, effort)
                    .await
                    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
                if send_starter {
                    spawn_starter_notice(bot, cid, tid, prov.to_string(), model.to_string());
                }
                return Ok(sess);
            }
            Err(e) => last_err = e,
        }
    }
    Err((StatusCode::BAD_GATEWAY, format!("Telegram topic creation failed: {last_err}")))
}

/// Best-effort sync of a topic rename (`editForumTopic`) to Telegram.
pub async fn sync_remote_topic_rename(ctx: &WebAppCtx, key: &SessionKey, new_name: &str) {
    let Some(tid) = key.topic_id.filter(|_| key.transport == "tg" && key.chat_id != 0) else {
        return;
    };
    let clean: String = new_name.trim().chars().take(120).collect();
    if clean.is_empty() {
        return;
    }
    ctx.topic_cache.insert(key.chat_id, tid, clean.clone());
    let Some(bot) = ctx.bot_clone().await else {
        return;
    };
    if let Ok(client) = reqwest::Client::builder().timeout(Duration::from_secs(8)).build() {
        let _ = client
            .post(bot_method_url(&bot, "editForumTopic"))
            .json(&serde_json::json!({
                "chat_id": key.chat_id,
                "message_thread_id": tid,
                "name": clean
            }))
            .send()
            .await;
    }
}

/// Best-effort deletion of a remote Telegram topic (`deleteForumTopic`).
pub async fn sync_remote_topic_delete(ctx: &WebAppCtx, key: &SessionKey) {
    let Some(tid) = key.topic_id.filter(|_| key.transport == "tg" && key.chat_id != 0) else {
        return;
    };
    let Some(bot) = ctx.bot_clone().await else {
        return;
    };
    if let Ok(client) = reqwest::Client::builder().timeout(Duration::from_secs(8)).build() {
        let _ = client
            .post(bot_method_url(&bot, "deleteForumTopic"))
            .json(&serde_json::json!({
                "chat_id": key.chat_id,
                "message_thread_id": tid
            }))
            .send()
            .await;
    }
}

/// Validates provider/model compatibility and creates either a `web` or `tg` session.
pub async fn create_session_from_payload(
    ctx: &WebAppCtx,
    payload: CreateSessionPayload,
) -> Result<SessionData, Box<Response>> {
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
        return Err(Box::new(json_err(StatusCode::BAD_REQUEST, "Model incompatible with provider")));
    }
    let name = payload.topic_name.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let res = if payload.transport.as_deref() == Some("tg") {
        create_remote_telegram_topic(
            ctx,
            name.as_deref(),
            payload.chat_id,
            prov,
            &model,
            payload.effort.as_deref(),
            !payload.silent_greeting,
        )
        .await
    } else {
        ctx.sessions
            .create_web_session(name, prov, &model, payload.effort.as_deref())
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))
    };
    res.map_err(|(code, msg)| Box::new(json_err(code, &msg)))
}
