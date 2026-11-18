//! # Bidirectional Telegram & Cron Event Bridge (`events_bridge.rs`)
//!
//! ## Overview
//! Provides compact async bridge helpers invoked by Telegram handlers, async observers,
//! and cron managers to publish unified [`WebEvent`] state transitions to connected Web
//! Dashboard clients with minimal call-site footprint.
//!
//! ## Search Tags
//! #event-bridge, #telegram-web-sync, #cron-events

use super::api_files::filter_safe_preview_paths;
use super::api_sessions::check_busy_and_asking;
use super::events::{emit_session_updated, publish_global, publish_global_delta, WebEvent};
use super::global_web_ctx;
use crate::cli::antigravity::session::AskState;
use crate::cli::AskQuestionData;
use crate::session::{data::SessionData, key::SessionKey};

fn make_key(transport: &str, cid: i64, tid: Option<i64>) -> SessionKey {
    SessionKey { transport: transport.to_string(), chat_id: cid, topic_id: tid }
}

/// Emits an idle `SessionUpdated` snapshot after model/provider/effort/`/new` mutations.
pub async fn emit_s(sess: &SessionData) {
    if let Some(ctx) = global_web_ctx() {
        let (busy, asking) = check_busy_and_asking(&ctx, sess).await;
        emit_session_updated(&ctx, sess, busy, asking).await;
    }
}

/// Emits a mirrored Telegram user prompt + `busy=true` snapshot.
pub async fn tg_user(sess: &SessionData, text: &str) {
    if let Some(ctx) = global_web_ctx() {
        ctx.events
            .publish(WebEvent::UserMessage {
                session_key: sess.session_key().storage_key(),
                source: "tg".to_string(),
                text: text.to_string(),
                timestamp: chrono::Utc::now().to_rfc3339(),
            })
            .await;
        emit_session_updated(&ctx, sess, true, false).await;
    }
}

/// Emits a throttled streaming delta for a Telegram session.
pub async fn tg_delta(cid: i64, tid: Option<i64>, text: &str) {
    publish_global_delta(&SessionKey::telegram(cid, tid).storage_key(), text, None).await;
}

/// Emits a throttled streaming delta for an async observer (`tg` or `web`).
pub async fn obs_delta(transport: &str, cid: i64, tid: Option<i64>, sid: &str, text: &str) {
    let sk = make_key(transport, cid, tid).storage_key();
    publish_global_delta(&sk, text, Some(sid.to_string())).await;
}

/// Emits final turn output and updates busy/asking state for `transport` (`tg` or `web`).
pub async fn emit_turn_res(
    transport: &str,
    cid: i64,
    tid: Option<i64>,
    sid: Option<String>,
    text: &str,
    is_error: bool,
) {
    let Some(ctx) = global_web_ctx() else { return };
    let key = make_key(transport, cid, tid);
    let files = filter_safe_preview_paths(&ctx, text);
    ctx.events
        .publish(WebEvent::TurnResult {
            session_key: key.storage_key(),
            session_id: sid,
            text: text.to_string(),
            is_error,
            files,
            timestamp: chrono::Utc::now().to_rfc3339(),
        })
        .await;
    if let Ok(Some(sess)) = ctx.sessions.get_active(&key).await {
        let (_, asking) = check_busy_and_asking(&ctx, &sess).await;
        emit_session_updated(&ctx, &sess, asking, asking).await;
    }
}

/// Emits a newly opened Telegram `AskQuestion` card state from `cli.sessions`.
pub async fn tg_ask(sess: &SessionData, sid: &str, prov: &str) {
    let Some(ctx) = global_web_ctx() else { return };
    let Some(state) = ctx.cli.sessions.get_ask_state(sid).await else { return };
    ctx.events
        .publish(WebEvent::AskQuestion {
            session_key: sess.session_key().storage_key(),
            session_id: sid.to_string(),
            provider: prov.to_string(),
            questions: state.questions,
            current_index: state.current_index,
            answers: state.answers,
        })
        .await;
    emit_session_updated(&ctx, sess, true, true).await;
}

/// Opens `AskQuestion` state when an async observer fires on a Web-only (`bot = None`) session.
pub async fn web_obs_ask(
    transport: &str,
    cid: i64,
    tid: Option<i64>,
    sid: &str,
    ask: Vec<AskQuestionData>,
) {
    let Some(ctx) = global_web_ctx() else { return };
    if ask.is_empty() {
        return;
    }
    let key = make_key(transport, cid, tid);
    let bmp = if ask[0].is_multi_select { "0".repeat(ask[0].options.len()) } else { String::new() };
    let n = ask.len();
    ctx.cli
        .sessions
        .set_ask_state(
            sid,
            AskState {
                msg_id: 0,
                questions: ask.clone(),
                current_index: 0,
                answers: vec![String::new(); n],
                current_bitmap: bmp,
                waiting_for_write_in: false,
            },
        )
        .await;
    ctx.events
        .publish(WebEvent::AskQuestion {
            session_key: key.storage_key(),
            session_id: sid.to_string(),
            provider: "antigravity".to_string(),
            questions: ask,
            current_index: 0,
            answers: vec![String::new(); n],
        })
        .await;
    if let Ok(Some(sess)) = ctx.sessions.get_active(&key).await {
        emit_session_updated(&ctx, &sess, true, true).await;
    }
}

/// Mirrors Telegram `AskQuestion` step navigation or completion by `(chat_id, session_id)`.
pub async fn tg_ask_by_sid(cid: i64, sid: &str, resolved: bool, summary: &str) {
    let Some(ctx) = global_web_ctx() else { return };
    let all = ctx.sessions.list_all().await.unwrap_or_default();
    let Some(sess) = all.into_iter().find(|s| {
        s.chat_id == cid && s.provider_sessions.values().any(|p| p.session_id == sid)
    }) else {
        return;
    };
    let sk = sess.session_key().storage_key();
    if resolved {
        ctx.events
            .publish(WebEvent::AskResolved {
                session_key: sk,
                session_id: sid.to_string(),
                summary: summary.to_string(),
            })
            .await;
        emit_session_updated(&ctx, &sess, true, false).await;
    } else {
        let prov = SessionData::effective_provider(&sess.provider, &ctx.config.provider);
        tg_ask(&sess, sid, prov).await;
    }
}

/// Clears busy/asking flags on connected Web clients after Telegram `/stop`.
pub async fn tg_stop(cid: i64, tid: Option<i64>, cleared_sids: &[String]) {
    let Some(ctx) = global_web_ctx() else { return };
    let key = SessionKey::telegram(cid, tid);
    let sk = key.storage_key();
    for sid in cleared_sids.iter().filter(|s| !s.is_empty()) {
        ctx.events
            .publish(WebEvent::AskResolved {
                session_key: sk.clone(),
                session_id: sid.clone(),
                summary: "Interrupted".to_string(),
            })
            .await;
    }
    if let Ok(Some(sess)) = ctx.sessions.get_active(&key).await {
        emit_session_updated(&ctx, &sess, false, false).await;
    }
}

/// Broadcasts idle state and ask-resolution for all persisted sessions after `/stop_all` or `/abort`.
pub async fn tg_stop_all(cleared_sids: &[String]) {
    let Some(ctx) = global_web_ctx() else { return };
    for sess in ctx.sessions.list_all().await.unwrap_or_default() {
        let sk = sess.session_key().storage_key();
        for ps in sess.provider_sessions.values() {
            if !ps.session_id.is_empty() && cleared_sids.contains(&ps.session_id) {
                ctx.events
                    .publish(WebEvent::AskResolved {
                        session_key: sk.clone(),
                        session_id: ps.session_id.clone(),
                        summary: "Interrupted".to_string(),
                    })
                    .await;
            }
        }
        emit_session_updated(&ctx, &sess, false, false).await;
    }
}

/// Emits [`WebEvent::CronUpdated`] non-blockingly when cron state changes.
pub fn cron_updated(job_id: &str, enabled: bool, last_run_status: Option<String>) {
    let jid = job_id.to_string();
    if tokio::runtime::Handle::try_current().is_ok() {
        tokio::spawn(async move {
            publish_global(WebEvent::CronUpdated { job_id: jid, enabled, last_run_status }).await;
        });
    }
}

/// Broadcasts [`WebEvent::HitlUpdated`] when a HITL remote browser route is registered or closed.
pub async fn hitl_updated(active: bool) {
    publish_global(WebEvent::HitlUpdated { active }).await;
}
