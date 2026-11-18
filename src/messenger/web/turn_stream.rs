//! # Web Streaming Turn Pump & Event Finalizer (`turn_stream.rs`)
//!
//! ## Overview
//! Consumes `StreamEvent` streams from CLI providers, broadcasts throttled deltas to
//! `WebEventHub`, mirrors Telegram topic live edits when `transport == "tg"`, opens interactive
//! `AskQuestion` states, and commits final transcripts and file previews.
//!
//! ## Search Tags
//! #web-stream, #telegram-mirror, #ask-stream-event, #turn-finalize

use super::api_files::filter_safe_preview_paths;
use super::events::{emit_session_updated, WebEvent};
use super::turn_runner::emit_web_error;
use super::WebAppCtx;
use crate::cli::antigravity::events::agy_state_root;
use crate::cli::antigravity::log_parser::AntigravityLogParser;
use crate::cli::antigravity::session::AskState;
use crate::cli::{AgentProvider, AskQuestionData, CliResponse, StreamEvent};
use crate::messenger::telegram::attachments::send_file_attachments;
use crate::messenger::telegram::commands_provider::{
    cancel_abandoned_provider_ask, is_stale_ask_session,
};
use crate::messenger::telegram::formatting::{markdown_to_telegram_html, split_html_message};
use crate::messenger::telegram::history::log_telegram_message;
use crate::messenger::telegram::message_session::sync_live_peer_ask_session;
use crate::messenger::telegram::session_init::configure_session_cli;
use crate::messenger::telegram::stream::{
    handle_text_delta, persist_stream_session_update, send_chunks_to_telegram,
};
use crate::session::data::SessionData;
use futures::StreamExt;
use std::sync::Arc;
use std::time::{Duration, Instant};
use teloxide::prelude::*;
use teloxide::types::MessageId;

struct StreamUiState {
    last_text: String,
    pub_id: Option<MessageId>,
    last_edit: Instant,
    last_sid: Option<String>,
    ask_opened: bool,
    pending_tg: bool,
}

impl StreamUiState {
    fn new() -> Self {
        Self {
            last_text: String::new(),
            pub_id: None,
            last_edit: Instant::now(),
            last_sid: None,
            ask_opened: false,
            pending_tg: false,
        }
    }
}

async fn handle_stream_delta(
    ctx: &Arc<WebAppCtx>,
    sess: &SessionData,
    sid: &str,
    bot_opt: Option<&Bot>,
    st: &mut StreamUiState,
    delta: &str,
) {
    let sk = sess.session_key().storage_key();
    ctx.events.publish_throttled_delta(&sk, delta, Some(sid.to_string()), 100).await;
    if let Some(bot) = bot_opt {
        let prev = st.last_edit;
        let tid = sess.topic_id.map(|t| t as i32);
        let _ = handle_text_delta(bot, ChatId(sess.chat_id), tid, delta, &mut st.last_text, &mut st.pub_id, &mut st.last_edit).await;
        st.pending_tg = st.pub_id.is_some() && st.last_edit == prev;
    }
}

async fn flush_pending_tg_edit(sess: &SessionData, bot_opt: Option<&Bot>, st: &mut StreamUiState) {
    if let Some(bot) = bot_opt {
        let cur = st.last_text.clone();
        let tid = sess.topic_id.map(|t| t as i32);
        let _ = handle_text_delta(bot, ChatId(sess.chat_id), tid, &cur, &mut st.last_text, &mut st.pub_id, &mut st.last_edit).await;
    }
    st.pending_tg = false;
}

async fn handle_single_stream_event(
    ctx: &Arc<WebAppCtx>,
    sess: &SessionData,
    sid: &str,
    bot_opt: Option<&Bot>,
    st: &mut StreamUiState,
    ev: StreamEvent,
) {
    match ev {
        StreamEvent::TextDelta(d) => handle_stream_delta(ctx, sess, sid, bot_opt, st, &d).await,
        StreamEvent::AskQuestion(ask) => {
            st.ask_opened = true;
            handle_web_ask_event(ctx, sess, bot_opt, ask).await;
        }
        StreamEvent::Result(resp) => {
            st.pending_tg = false;
            st.last_sid = finalize_web_result(ctx, sess, bot_opt, st.pub_id, resp).await;
        }
    }
}

fn capture_pre_transcript_cursor(
    cc: &crate::cli::antigravity::AntigravityCli,
    sess: &SessionData,
    sid: &str,
    default_prov: &str,
) -> Option<(std::path::PathBuf, u64)> {
    let prov = SessionData::effective_provider(&sess.provider, default_prov);
    if prov != "antigravity" || sid.is_empty() {
        return None;
    }
    let tp = agy_state_root(Some(&cc.build_env())).join("brain").join(sid).join(".system_generated/logs/transcript_full.jsonl");
    let pre_len = std::fs::metadata(&tp).map(|m| m.len()).unwrap_or(0);
    Some((tp, pre_len))
}

async fn recover_missed_agy_ask(
    ctx: &Arc<WebAppCtx>,
    sess: &SessionData,
    bot_opt: Option<&Bot>,
    pre: Option<&(std::path::PathBuf, u64)>,
    st: &mut StreamUiState,
) {
    if st.ask_opened {
        return;
    }
    let Some((tp, pre_len)) = pre else { return };
    let delta = AntigravityLogParser::new().parse_log_delta_structured(tp, Some(*pre_len));
    if let Some(ask) = delta.ask_question {
        st.ask_opened = true;
        handle_web_ask_event(ctx, sess, bot_opt, ask).await;
    }
}

/// Streams a prompt (or resumed post-`AskQuestion` turn with empty prompt) for Web & Telegram.
pub async fn stream_web_turn(ctx: &Arc<WebAppCtx>, sess: &SessionData, sid: &str, prompt: &str) {
    let cc = configure_session_cli(&ctx.cli, sess, sess.chat_id, sess.topic_id).await;
    let pre_cursor = capture_pre_transcript_cursor(&cc, sess, sid, &ctx.config.provider);
    let stream_res = cc.send_streaming(prompt, (!sid.is_empty()).then_some(sid), false, cc.config.working_dir.clone()).await;
    let mut stream = match stream_res {
        Ok(s) => s,
        Err(e) => {
            let err_msg = crate::cli::antigravity::error_parser::parse_cli_error(&e, 1, cc.config.model.as_deref());
            emit_web_error(ctx, sess, &err_msg).await;
            return;
        }
    };
    let bot_opt = if sess.transport == "tg" && sess.chat_id != 0 { ctx.bot_clone().await } else { None };
    let mut st = StreamUiState::new();
    let mut tick = tokio::time::interval(Duration::from_millis(700));
    loop {
        tokio::select! {
            ev = stream.next() => match ev {
                Some(e) => handle_single_stream_event(ctx, sess, sid, bot_opt.as_ref(), &mut st, e).await,
                None => break,
            },
            _ = tick.tick(), if st.pending_tg && st.last_edit.elapsed() >= Duration::from_secs(2) => {
                flush_pending_tg_edit(sess, bot_opt.as_ref(), &mut st).await;
            }
        }
    }
    recover_missed_agy_ask(ctx, sess, bot_opt.as_ref(), pre_cursor.as_ref(), &mut st).await;
    finish_stream_state(ctx, sess, sid, st).await;
}

async fn finish_stream_state(ctx: &Arc<WebAppCtx>, sess: &SessionData, sid: &str, st: StreamUiState) {
    if st.last_sid.is_some() {
        persist_stream_session_update(&ctx.sessions, sess, sess.clone(), st.last_sid.clone(), &ctx.config).await;
    }
    let latest = ctx.sessions.get_active(&sess.session_key()).await.ok().flatten();
    let eff_sess = latest.as_ref().unwrap_or(sess);
    let eff_sid = st.last_sid.as_deref().unwrap_or(sid);
    let asking = st.ask_opened && ctx.cli.sessions.is_ask_active(eff_sid).await;
    emit_session_updated(ctx, eff_sess, false, asking).await;
}

async fn handle_web_ask_event(
    ctx: &Arc<WebAppCtx>,
    sess: &SessionData,
    bot_opt: Option<&Bot>,
    ask: Vec<AskQuestionData>,
) {
    if ask.is_empty() {
        return;
    }
    let prov = SessionData::effective_provider(&sess.provider, &ctx.config.provider);
    let eff = sync_live_peer_ask_session(&ctx.sessions, &ctx.config, sess, prov, sess.chat_id).await;
    let sid = eff.get_session_id(prov);
    if is_stale_ask_session(&ctx.sessions, &ctx.config, &eff, prov, &sid).await {
        cancel_abandoned_provider_ask(&ctx.cli, prov, sess.chat_id, eff.topic_id, &sid).await;
        return;
    }
    let mut tg_msg_id = 0_i32;
    if let Some(bot) = bot_opt
        && let Ok(mid) = crate::messenger::telegram::ask_process::handle_stream_ask_question(
            bot, ChatId(eff.chat_id), eff.topic_id.map(|t| t as i32), ask[0].clone(), &eff, &ctx.config,
        )
        .await
    {
        tg_msg_id = mid;
    }
    let bmp = if ask[0].is_multi_select { "0".repeat(ask[0].options.len()) } else { String::new() };
    let n = ask.len();
    let ask_state = AskState {
        msg_id: tg_msg_id,
        questions: ask.clone(),
        current_index: 0,
        answers: vec![String::new(); n],
        current_bitmap: bmp,
        waiting_for_write_in: false,
    };
    ctx.cli.sessions.set_ask_state(&sid, ask_state).await;
    ctx.events
        .publish(WebEvent::AskQuestion {
            session_key: eff.session_key().storage_key(),
            session_id: sid,
            provider: prov.to_string(),
            questions: ask,
            current_index: 0,
            answers: vec![String::new(); n],
        })
        .await;
    emit_session_updated(ctx, &eff, true, true).await;
}

async fn finalize_web_result(
    ctx: &Arc<WebAppCtx>,
    sess: &SessionData,
    bot_opt: Option<&Bot>,
    pub_id: Option<MessageId>,
    resp: CliResponse,
) -> Option<String> {
    let last_sid = resp.session_id.clone();
    if resp.stderr == "Interrupted by user (/stop)"
        || (!resp.is_error && resp.result.trim().is_empty())
    {
        return last_sid;
    }
    let raw_text = if resp.is_error {
        let err_msg = if !resp.stderr.is_empty() { &resp.stderr } else { &resp.result };
        crate::cli::antigravity::error_parser::parse_cli_error(err_msg, resp.returncode.unwrap_or(1), ctx.config.model.as_deref())
    } else {
        resp.result
    };
    let mut sent_id = None;
    if let Some(bot) = bot_opt {
        let html = markdown_to_telegram_html(&raw_text);
        let chunks = split_html_message(&html, 4000);
        let tid = sess.topic_id.map(|t| t as i32);
        let (_, _, sid_opt) = send_chunks_to_telegram(bot, ChatId(sess.chat_id), pub_id, tid, &chunks).await;
        sent_id = sid_opt;
        send_file_attachments(bot, ChatId(sess.chat_id), tid, &raw_text, &ctx.config).await;
    }
    if let Some(ref sid) = last_sid {
        log_telegram_message(&ctx.working_dir, sid, (sess.topic_id, sess.topic_name.as_deref()), "bot", sent_id, &raw_text, (!resp.is_error, resp.is_error.then_some(raw_text.as_str())));
    }
    let files = filter_safe_preview_paths(ctx, &raw_text);
    ctx.events
        .publish(WebEvent::TurnResult {
            session_key: sess.session_key().storage_key(),
            session_id: last_sid.clone(),
            text: raw_text,
            is_error: resp.is_error,
            files,
            timestamp: chrono::Utc::now().to_rfc3339(),
        })
        .await;
    last_sid
}

#[cfg(test)]
pub(crate) async fn drive_stream_events_for_test(
    ctx: &Arc<WebAppCtx>,
    sess: &SessionData,
    sid: &str,
    events: Vec<StreamEvent>,
) {
    let mut st = StreamUiState::new();
    for ev in events {
        handle_single_stream_event(ctx, sess, sid, None, &mut st, ev).await;
    }
    finish_stream_state(ctx, sess, sid, st).await;
}
