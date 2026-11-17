//! # Web Turn Execution & Telegram Mirroring Dispatch (`turn_runner.rs`)
//!
//! ## Overview
//! Acquires per-topic `LockPool` exclusivity for Web-initiated turns, resolves inline
//! `@model` directives, drains pending uploads, mirrors `[🌐 Web]` messages into Telegram
//! forum topics when mirrored, and delegates stream consumption to [`super::turn_stream`].
//!
//! ## Search Tags
//! #web-turn-runner, #lock-pool, #model-directive, #telegram-mirror

use super::events::{emit_session_updated, WebEvent};
use super::turn_boot::ensure_web_session_ready_for_turn;
use super::turn_stream::stream_web_turn;
use super::WebAppCtx;
use crate::messenger::telegram::commands_model::apply_model_to_active_pty;
use crate::messenger::telegram::commands_registry::escape_non_workflow_slash;
use crate::messenger::telegram::formatting::{markdown_to_telegram_html, split_html_message};
use crate::messenger::telegram::handler::normalize_command_text;
use crate::messenger::telegram::history::log_telegram_message;
use crate::messenger::telegram::media::inject_pre_downloaded_files;
use crate::messenger::telegram::reply_directives::{
    resolve_directive_for_provider, DirectiveOutcome,
};
use crate::messenger::telegram::stream::send_chunks_to_telegram;
use crate::session::{data::SessionData, key::SessionKey};
use std::sync::Arc;
use teloxide::prelude::*;
use teloxide::types::{MessageId, ParseMode, ThreadId};

/// Sends `markdown` to the mirrored Telegram topic when `sess.transport == "tg"`.
pub(crate) async fn mirror_text_to_telegram(
    ctx: &WebAppCtx,
    sess: &SessionData,
    markdown: &str,
) -> Option<i32> {
    if sess.transport != "tg" {
        return None;
    }
    let bot = ctx.bot_clone().await?;
    let cid = ChatId(sess.chat_id);
    let tid = sess.topic_id.map(|t| t as i32);
    let html = markdown_to_telegram_html(markdown);
    let chunks = split_html_message(&html, 4000);
    let (_, _, sent_id) = send_chunks_to_telegram(&bot, cid, None, tid, &chunks).await;
    sent_id
}

pub(crate) async fn live_open_ask_sid(ctx: &WebAppCtx, sess: &SessionData) -> Option<String> {
    let prov = SessionData::effective_provider(&sess.provider, &ctx.config.provider);
    let sid = sess.get_session_id(prov);
    if sid.is_empty() || !ctx.cli.sessions.is_ask_active(&sid).await {
        return None;
    }
    if sid.starts_with("mock-") || ctx.cli.is_ask_backing_alive(prov, sess.chat_id, sess.topic_id, &sid).await {
        Some(sid)
    } else {
        ctx.cli.sessions.set_ask_active(&sid, false).await;
        None
    }
}

/// Emits a turn error result and clears busy flags on connected Web clients while preserving open ask state.
pub(crate) async fn emit_web_error(ctx: &Arc<WebAppCtx>, sess: &SessionData, msg: &str) {
    let sk = sess.session_key().storage_key();
    let _ = mirror_text_to_telegram(ctx, sess, msg).await;
    ctx.events
        .publish(WebEvent::TurnResult {
            session_key: sk,
            session_id: None,
            text: msg.to_string(),
            is_error: true,
            files: Vec::new(),
            timestamp: chrono::Utc::now().to_rfc3339(),
        })
        .await;
    let asking = live_open_ask_sid(ctx, sess).await.is_some();
    emit_session_updated(ctx, sess, false, asking).await;
}

async fn persist_web_directive(ctx: &WebAppCtx, sess: &mut SessionData, prov: &str, m: &str, eff: Option<String>) {
    if let Ok(Some(u)) = ctx
        .sessions
        .mutate_session(&sess.session_key(), Some(sess), |cur| {
            cur.provider = prov.to_string();
            cur.model = m.to_string();
            cur.effort = eff.clone();
            true
        })
        .await
    {
        *sess = u;
    }
    apply_model_to_active_pty(&ctx.cli, &ctx.sessions, sess).await;
}

async fn apply_web_directive<'a>(
    ctx: &Arc<WebAppCtx>,
    sess: &mut SessionData,
    raw_text: &'a str,
    ask_open: bool,
) -> Option<&'a str> {
    let prov = SessionData::effective_provider(&sess.provider, &ctx.config.provider).to_string();
    if raw_text.trim_start().starts_with('@') && !matches!(prov.as_str(), "claude" | "codex") {
        crate::cli::antigravity::model_spec::ensure_live_catalog_fresh().await;
    }
    let prev_eff = sess.effort.clone();
    match resolve_directive_for_provider(&prov, prev_eff.as_deref(), raw_text) {
        DirectiveOutcome::InvalidEffort => {
            emit_web_error(ctx, sess, "❌ Invalid effort level.").await;
            None
        }
        DirectiveOutcome::IncompatibleModel(m) => {
            emit_web_error(ctx, sess, &format!("⚠️ Model `{m}` is incompatible with `{prov}`.")).await;
            None
        }
        DirectiveOutcome::Resolved(canon_m, eff, rem) => {
            if ask_open && !rem.is_empty() {
                emit_web_error(ctx, sess, "⚠️ Active question awaiting answer — please answer, skip, or /stop first.").await;
                return None;
            }
            persist_web_directive(ctx, sess, &prov, &canon_m, eff).await;
            emit_session_updated(ctx, sess, !rem.is_empty(), ask_open).await;
            if rem.is_empty() {
                emit_web_error(ctx, sess, &format!("🤖 [tuner] Next message will use `{canon_m}`.")).await;
                return None;
            }
            Some(rem)
        }
        DirectiveOutcome::None => Some(raw_text.trim()),
    }
}

async fn queue_web_attachments(ctx: &WebAppCtx, sess: &mut SessionData, extra: &[String]) {
    let valid: Vec<String> = extra.iter().filter(|f| !f.trim().is_empty()).cloned().collect();
    if valid.is_empty() {
        return;
    }
    if let Ok(Some(u)) = ctx
        .sessions
        .mutate_session(&sess.session_key(), None, |cur| {
            let mut changed = false;
            for f in &valid {
                if !cur.pending_attachments.contains(f) {
                    cur.pending_attachments.push(f.clone());
                    changed = true;
                }
            }
            changed
        })
        .await
    {
        *sess = u;
    }
}

async fn try_route_open_ask_turn(
    ctx: &Arc<WebAppCtx>,
    sess: &mut SessionData,
    raw_text: &str,
    normalized: &str,
    extra_files: &[String],
) -> bool {
    let Some(ask_sid) = live_open_ask_sid(ctx, sess).await else {
        return false;
    };
    queue_web_attachments(ctx, sess, extra_files).await;
    if normalized.trim_start().starts_with('@')
        && apply_web_directive(ctx, sess, normalized, true).await.is_none()
    {
        return true;
    }
    if raw_text.trim().is_empty() {
        emit_session_updated(ctx, sess, false, true).await;
        return true;
    }
    super::api_ask::answer_open_ask_under_lock(ctx, sess, &ask_sid, raw_text.trim()).await;
    true
}

/// Runs a complete user prompt turn on `key` from the Web Dashboard.
pub async fn execute_web_turn(
    ctx: Arc<WebAppCtx>,
    key: SessionKey,
    raw_text: String,
    extra_files: Vec<String>,
) {
    let lock = ctx.sessions.lock_pool.get((key.chat_id, key.topic_id));
    let _topic_guard = lock.lock().await;
    let Ok(Some(mut sess)) = ctx.sessions.get_active(&key).await else {
        return;
    };
    let normalized = normalize_command_text(&raw_text, None);
    if try_route_open_ask_turn(&ctx, &mut sess, &raw_text, &normalized, &extra_files).await {
        return;
    }
    let Some(prompt_body) = apply_web_directive(&ctx, &mut sess, &normalized, false).await else {
        return;
    };
    emit_session_updated(&ctx, &sess, true, false).await;
    let sid = match ensure_web_session_ready_for_turn(&ctx, &mut sess, false).await {
        Ok(s) if !s.is_empty() => s,
        Ok(_) => {
            emit_session_updated(&ctx, &sess, false, false).await;
            return;
        }
        Err(e) => {
            emit_web_error(&ctx, &sess, &e).await;
            return;
        }
    };
    let mut files = drain_web_attachments(&ctx, &mut sess, &extra_files).await;
    files.retain(|f| !f.trim().is_empty());
    let mut full_prompt = escape_non_workflow_slash(prompt_body);
    inject_pre_downloaded_files(&mut full_prompt, &files);
    log_and_mirror_web_prompt(&ctx, &sess, &key, &sid, &raw_text).await;
    let _fg = crate::upgrade::begin_foreground_run();
    let _turn_g = ctx.cli.begin_topic_turn(key.chat_id, key.topic_id);
    stream_web_turn(&ctx, &sess, &sid, &full_prompt).await;
}

async fn log_and_mirror_web_prompt(ctx: &WebAppCtx, sess: &SessionData, key: &SessionKey, sid: &str, raw_text: &str) {
    log_telegram_message(&ctx.working_dir, sid, (sess.topic_id, sess.topic_name.as_deref()), "user", None, raw_text, (true, None));
    ctx.events
        .publish(WebEvent::UserMessage {
            session_key: key.storage_key(),
            source: "web".to_string(),
            text: raw_text.to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
        })
        .await;
    if sess.transport == "tg" {
        let _ = mirror_text_to_telegram(ctx, sess, &format!("🌐 [Web] {raw_text}")).await;
    }
}

async fn drain_web_attachments(
    ctx: &WebAppCtx,
    sess: &mut SessionData,
    extra: &[String],
) -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(Some(u)) = ctx
        .sessions
        .mutate_session(&sess.session_key(), None, |cur| {
            if cur.pending_attachments.is_empty() {
                return false;
            }
            out = std::mem::take(&mut cur.pending_attachments);
            true
        })
        .await
    {
        *sess = u;
    }
    for f in extra {
        if !out.contains(f) {
            out.push(f.clone());
        }
    }
    out
}

/// Mirrors a Telegram `AskQuestion` UI update when resolved or stepped from the Web Dashboard.
pub async fn mirror_ask_message_to_telegram(
    ctx: &WebAppCtx,
    sess: &SessionData,
    msg_id: i32,
    markdown: &str,
    markup: Option<teloxide::types::InlineKeyboardMarkup>,
) {
    if sess.transport != "tg" || msg_id == 0 {
        return;
    }
    let Some(bot) = ctx.bot_clone().await else {
        return;
    };
    let html = markdown_to_telegram_html(markdown);
    let mut req = bot
        .edit_message_text(ChatId(sess.chat_id), MessageId(msg_id), html)
        .parse_mode(ParseMode::Html);
    if let Some(rm) = markup {
        req = req.reply_markup(rm);
    }
    let _ = req.await;
    let _ = ThreadId(MessageId(0));
}
