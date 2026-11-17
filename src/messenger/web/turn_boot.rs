//! # Web Session Boot & Auto-Handover Helper (`turn_boot.rs`)
//!
//! ## Overview
//! Ensures Web and unified topics have a live provider session ID before dispatching turns,
//! synthesizing context handover summaries from prior topic transcripts on fresh boots and
//! attaching background async observers for `antigravity`.
//!
//! ## Search Tags
//! #web-session-boot, #auto-handover, #async-observer

use super::events::WebEvent;
use super::turn_runner::mirror_text_to_telegram;
use super::WebAppCtx;
use crate::cli::{AgentProvider, CliResponse};
use crate::messenger::telegram::handover::build_topic_handover;
use crate::messenger::telegram::history::log_telegram_message;
use crate::messenger::telegram::session_init::configure_session_cli;
use crate::session::data::SessionData;
use std::sync::Arc;

async fn resolve_auto_handover_web(ctx: &WebAppCtx, sess: &mut SessionData) -> Option<String> {
    let prov = SessionData::effective_provider(&sess.provider, &ctx.config.provider);
    let past_ids = sess.get_past_session_ids(prov);
    if past_ids.is_empty() && sess.last_handover_summary.is_none() {
        return None;
    }
    let last_sid = past_ids.last().map(String::as_str).unwrap_or("");
    let handover = build_topic_handover(
        &ctx.working_dir,
        last_sid,
        sess.topic_id,
        sess.topic_name.clone(),
        sess.last_handover_summary.as_deref(),
        &past_ids,
    )?;
    let summary = format!(
        "Objective: {}\nDecisions: {}\nProgress: {}",
        handover.topic_objective,
        handover.agreed_decisions.join("; "),
        handover.current_progress
    );
    if let Ok(Some(updated)) = ctx
        .sessions
        .mutate_session(&sess.session_key(), Some(sess), |cur| {
            cur.last_handover_summary = Some(summary.clone());
            true
        })
        .await
    {
        *sess = updated;
    }
    Some(handover.handover_prompt)
}

fn spawn_web_async_observer(ctx: &Arc<WebAppCtx>, sess: &SessionData, sid: &str) {
    let prov = SessionData::effective_provider(&sess.provider, &ctx.config.provider);
    if prov != "antigravity" || sid.is_empty() || cfg!(test) {
        return;
    }
    let bot_opt = if sess.transport == "tg" && sess.chat_id != 0 {
        ctx.bot.try_read().ok().and_then(|g| g.clone())
    } else {
        None
    };
    crate::messenger::telegram::async_observer::spawn_session_async_observer_target(
        bot_opt,
        (sess.chat_id, sess.topic_id, sess.transport.clone()),
        sid.to_string(),
        (*ctx.cli).clone(),
        ctx.sessions.clone(),
        (*ctx.config).clone(),
    );
}

/// Ensures `sess` has a live CLI session ID (booting with handover summary if needed).
pub async fn ensure_web_session_ready_for_turn(
    ctx: &Arc<WebAppCtx>,
    sess: &mut SessionData,
    announce_boot: bool,
) -> Result<String, String> {
    let prov = SessionData::effective_provider(&sess.provider, &ctx.config.provider);
    let existing = sess.get_session_id(prov);
    if !existing.is_empty() {
        let mut check_cli = (*ctx.cli).clone();
        check_cli.config.provider = prov.to_string();
        if existing.starts_with("mock-") || check_cli.is_session_alive(&existing) {
            spawn_web_async_observer(ctx, sess, &existing);
            return Ok(existing);
        }
        let _ = ctx
            .sessions
            .mutate_session(&sess.session_key(), Some(sess), |cur| {
                if cur.get_session_id(prov) == existing {
                    cur.set_session_id(prov, "");
                    return true;
                }
                false
            })
            .await;
    }
    let boot_sid = boot_fresh_web_session(ctx, sess, announce_boot).await?;
    if !boot_sid.is_empty() {
        spawn_web_async_observer(ctx, sess, &boot_sid);
    }
    Ok(boot_sid)
}

async fn run_boot_send(
    ctx: &Arc<WebAppCtx>,
    sess: &SessionData,
    init_prompt: &str,
) -> Result<Option<CliResponse>, String> {
    let _turn_guard = ctx.cli.begin_topic_turn(sess.chat_id, sess.topic_id);
    let session_cli = configure_session_cli(&ctx.cli, sess, sess.chat_id, sess.topic_id).await;
    let ws = session_cli.agy_workspace();
    let mut cancel_rx = ctx.cli.sessions.register_boot(sess.chat_id, sess.topic_id).await;
    let send_fut = session_cli.send(init_prompt, None, false, ws);
    tokio::pin!(send_fut);
    let res = tokio::select! {
        r = &mut send_fut => {
            ctx.cli.sessions.unregister_boot(sess.chat_id, sess.topic_id).await;
            r?
        }
        Ok(()) = &mut cancel_rx => {
            ctx.cli.sessions.unregister_boot(sess.chat_id, sess.topic_id).await;
            return Ok(None);
        }
    };
    if res.is_error {
        return Err(if res.stderr.is_empty() { res.result } else { res.stderr });
    }
    Ok(Some(res))
}

async fn persist_and_announce_boot(
    ctx: &Arc<WebAppCtx>,
    sess: &mut SessionData,
    new_sid: &str,
    result_text: &str,
    announce: bool,
) {
    let boot_prov = SessionData::effective_provider(&sess.provider, &ctx.config.provider);
    if let Ok(Some(u)) = ctx
        .sessions
        .mutate_session(&sess.session_key(), Some(sess), |cur| {
            if cur.get_session_id(boot_prov).is_empty() {
                cur.set_session_id(boot_prov, new_sid);
                cur.last_active = chrono::Utc::now().to_rfc3339();
                return true;
            }
            false
        })
        .await
    {
        *sess = u;
    }
    let greeting = if result_text.trim().is_empty() {
        crate::t!("bot.new_session")
    } else {
        result_text.to_string()
    };
    let sent_id = if announce { mirror_text_to_telegram(ctx, sess, &greeting).await } else { None };
    log_telegram_message(&ctx.working_dir, new_sid, (sess.topic_id, sess.topic_name.as_deref()), "bot", sent_id, &greeting, (true, None));
    if announce {
        ctx.events
            .publish(WebEvent::TurnResult {
                session_key: sess.session_key().storage_key(),
                session_id: Some(new_sid.to_string()),
                text: greeting,
                is_error: false,
                files: Vec::new(),
                timestamp: chrono::Utc::now().to_rfc3339(),
            })
            .await;
    }
}

async fn boot_fresh_web_session(
    ctx: &Arc<WebAppCtx>,
    sess: &mut SessionData,
    announce_boot: bool,
) -> Result<String, String> {
    let handover = resolve_auto_handover_web(ctx, sess).await;
    let default_prompt = crate::t!("bot.session_init_prompt");
    let init_prompt = handover.as_deref().unwrap_or(&default_prompt);
    if cfg!(test) {
        let mock_sid = format!("mock-web-{}", chrono::Utc::now().timestamp_millis());
        persist_and_announce_boot(ctx, sess, &mock_sid, "", false).await;
        return Ok(mock_sid);
    }
    let Some(res) = run_boot_send(ctx, sess, init_prompt).await? else {
        return Ok(String::new());
    };
    let Some(new_sid) = res.session_id.clone() else {
        return Err("Session initialization did not return a session_id".to_string());
    };
    let announce = announce_boot || handover.is_some();
    persist_and_announce_boot(ctx, sess, &new_sid, &res.result, announce).await;
    Ok(new_sid)
}
