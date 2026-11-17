//! # Streaming Output Consumer for Telegram Chat
//!
//! ## Overview
//! Consumes character/event streams from active agent CLI PTY processes. Debounces text chunks
//! to minimize Telegram message update rate-limiting and renders real-time outputs.
//!
//! ## Search Tags
//! #streaming-consumer, #debouncer, #rate-limiting, #message-updates

use teloxide::prelude::*;
use teloxide::types::MessageId;
use crate::config::CliConfig;
use crate::cli::StreamEvent;
use crate::session::{data::SessionData, key::SessionKey, manager::SessionManager};
use std::time::{Instant, Duration};
use futures::StreamExt;
use super::formatting;
use crate::messenger::web::events_bridge as wev;
pub(crate) use super::rate_limiter::{is_message_not_modified, send_chunks_to_telegram, send_single_chunk};

pub(crate) async fn handle_stream_result(
    bot: &Bot,
    chat_id: ChatId,
    msg_id: Option<MessageId>,
    thread_id: Option<i32>,
    topic: (Option<i64>, Option<&str>),
    resp: crate::cli::CliResponse,
    config: &CliConfig,
) -> Result<Option<String>, teloxide::RequestError> {
    let last_sid = resp.session_id.clone();
    if resp.stderr == "Interrupted by user (/stop)" || (!resp.is_error && resp.result.trim().is_empty()) {
        return Ok(last_sid);
    }
    let is_err = resp.is_error;
    let raw_text = if is_err {
        let err_msg = if !resp.stderr.is_empty() { &resp.stderr } else { &resp.result };
        crate::cli::antigravity::error_parser::parse_cli_error(err_msg, resp.returncode.unwrap_or(1), config.model.as_deref())
    } else {
        resp.result
    };
    let chunks = formatting::split_html_message(&formatting::markdown_to_telegram_html(&raw_text), 4000);
    let (ok, err, sent_id) = send_chunks_to_telegram(bot, chat_id, msg_id, thread_id, &chunks).await;
    let _ = super::attachments::send_file_attachments(bot, chat_id, thread_id, &raw_text, config).await;
    if let Some(ref sid) = last_sid {
        super::history::log_telegram_message(&config.working_dir, sid, topic, "bot", sent_id, &raw_text, (ok, err.as_deref()));
    }
    let tid64 = thread_id.map(|t| t as i64).or(topic.0);
    wev::emit_turn_res("tg", chat_id.0, tid64, last_sid.clone(), &raw_text, is_err).await;
    Ok(last_sid)
}

fn truncate_streaming_preview(delta: &str) -> String {
    if delta.encode_utf16().count() > 3500 {
        let mut units = 0;
        let tail: Vec<char> = delta.chars().rev().take_while(|c| {
            units += c.len_utf16();
            units <= 3400
        }).collect();
        let tail_str: String = tail.into_iter().rev().collect();
        let aligned = tail_str.find('\n').map(|i| &tail_str[i + 1..]).unwrap_or(&tail_str);
        format!("...[중간 생략]...\n{}", aligned)
    } else {
        delta.to_string()
    }
}

pub(crate) fn render_streaming_preview_html(delta: &str) -> String {
    let full_html = formatting::markdown_to_telegram_html(&truncate_streaming_preview(delta));
    formatting::split_html_message(&full_html, 3800).into_iter().last().unwrap_or(full_html)
}

pub(crate) async fn handle_text_delta(
    bot: &Bot,
    chat_id: ChatId,
    thread_id: Option<i32>,
    delta: &str,
    last_text: &mut String,
    pub_msg_id: &mut Option<MessageId>,
    last_edit: &mut Instant,
) -> Result<(), teloxide::RequestError> {
    *last_text = delta.to_string();
    if pub_msg_id.is_some() && last_edit.elapsed() < Duration::from_secs(2) {
        return Ok(());
    }
    let html = render_streaming_preview_html(delta);
    match send_single_chunk(bot, chat_id, *pub_msg_id, thread_id, &html, true).await {
        Ok(mid) => {
            *pub_msg_id = Some(mid);
            *last_edit = Instant::now();
        }
        Err(e) if !is_message_not_modified(&e) => {
            eprintln!("❌ [tuner] Stream msg failed ({}, {:?}): {:?}", chat_id, *pub_msg_id, e);
        }
        _ => {}
    }
    Ok(())
}

struct StreamLoopState {
    last_text: String,
    pub_id: Option<MessageId>,
    last_sid: Option<String>,
}

type StreamCtx<'a> = (
    &'a CliConfig,
    &'a crate::cli::antigravity::AntigravityCli,
    &'a crate::session::manager::SessionManager,
);

async fn process_stream_events(
    bot: &Bot,
    chat_id: ChatId,
    tid: Option<i32>,
    mut stream: futures::stream::BoxStream<'_, StreamEvent>,
    sdata: &crate::session::data::SessionData,
    ctx: StreamCtx<'_>,
    state: &mut StreamLoopState,
) -> Result<(), teloxide::RequestError> {
    let (cfg, cli, sessions) = ctx;
    let (mut last_edit, mut pending) = (Instant::now(), false);
    let mut tick = tokio::time::interval(Duration::from_millis(700));
    loop {
        tokio::select! {
            ev = stream.next() => match ev {
                Some(StreamEvent::TextDelta(d)) => {
                    let prev = last_edit;
                    wev::tg_delta(chat_id.0, sdata.topic_id, &d).await;
                    handle_text_delta(bot, chat_id, tid, &d, &mut state.last_text, &mut state.pub_id, &mut last_edit).await?;
                    pending = state.pub_id.is_some() && last_edit == prev;
                }
                Some(StreamEvent::AskQuestion(ask)) => {
                    super::ask_process::handle_ask_question_event(bot, chat_id, tid, ask, sdata, (cfg, cli, sessions)).await?
                }
                Some(StreamEvent::Result(resp)) => {
                    pending = false;
                    if !resp.result.is_empty() { state.last_text = resp.result.clone(); }
                    let topic = (sdata.topic_id, sdata.topic_name.as_deref());
                    if let Ok(Some(sid)) = handle_stream_result(bot, chat_id, state.pub_id, tid, topic, resp, cfg).await {
                        state.last_sid = Some(sid);
                    }
                }
                None => break,
            },
            _ = tick.tick(), if pending && last_edit.elapsed() >= Duration::from_secs(2) => {
                let cur = state.last_text.clone();
                handle_text_delta(bot, chat_id, tid, &cur, &mut state.last_text, &mut state.pub_id, &mut last_edit).await?;
                pending = false;
            }
        }
    }
    Ok(())
}

pub(crate) async fn persist_stream_session_update(
    sessions: &SessionManager,
    session_data: &SessionData,
    updated: SessionData,
    last_sid: Option<String>,
    config: &CliConfig,
) {
    let turn_prov = SessionData::effective_provider(&session_data.provider, &config.provider);
    let start_sid = session_data.get_session_id(turn_prov);
    let new_prog_mid = updated.last_progress_msg_id;
    let fallback = start_sid.is_empty().then_some(&updated);
    let _ = sessions
        .mutate_session(&session_data.session_key(), fallback, |cur| {
            let latest_sid = cur.get_session_id(turn_prov);
            let sid_diverged = latest_sid != start_sid
                && Some(latest_sid.as_str()) != last_sid.as_deref();
            let was_reset = (!start_sid.is_empty()
                && (cur.get_past_session_ids(turn_prov).contains(&start_sid) || sid_diverged))
                || (start_sid.is_empty() && !latest_sid.is_empty() && sid_diverged);
            if was_reset {
                return false;
            }
            cur.last_progress_msg_id = new_prog_mid;
            if let Some(ref sid) = last_sid {
                cur.set_session_id(turn_prov, sid);
            }
            cur.increment_message_count(turn_prov);
            cur.last_active = chrono::Utc::now().to_rfc3339();
            true
        })
        .await;
}

pub(crate) async fn persist_async_antigravity_turn(
    sessions: &SessionManager,
    key: &SessionKey,
    agy_sid: &str,
    _default_provider: &str,
) -> Option<String> {
    let mut topic_name = None;
    let _ = sessions
        .mutate_session(key, None, |cur| {
            topic_name = cur.topic_name.clone();
            if cur.get_session_id("antigravity") != agy_sid
                || cur.get_past_session_ids("antigravity").iter().any(|p| p == agy_sid)
            {
                return false;
            }
            cur.increment_message_count("antigravity");
            cur.last_active = chrono::Utc::now().to_rfc3339();
            true
        })
        .await;
    topic_name
}

pub(crate) fn build_observer_ask_session(
    active: Option<SessionData>,
    chat_id: i64,
    topic_id: Option<i64>,
    agy_sid: &str,
) -> SessionData {
    let mut sess_data = active.unwrap_or_else(|| SessionData {
        chat_id,
        topic_id,
        ..Default::default()
    });
    sess_data.provider = "antigravity".to_string();
    sess_data.set_session_id("antigravity", agy_sid);
    sess_data
}

pub(crate) async fn consume_stream(
    bot: &Bot,
    target: (ChatId, Option<i32>),
    stream: futures::stream::BoxStream<'_, StreamEvent>,
    sessions: &crate::session::manager::SessionManager,
    session_data: crate::session::data::SessionData,
    config: &CliConfig,
    cli: &crate::cli::antigravity::AntigravityCli,
) -> Result<(), teloxide::RequestError> {
    let (chat_id, thread_id) = target;
    let mut state = StreamLoopState {
        last_text: String::new(),
        pub_id: None,
        last_sid: None,
    };
    let mut updated = session_data.clone();
    let mut cleared = false;
    if let Some(last_mid) = session_data.last_progress_msg_id {
        let tok = std::env::var("TELEGRAM_TOKEN").unwrap_or_else(|_| config.telegram_token.clone());
        super::typing::clear_old_progress_reaction(last_mid, chat_id, tok);
        updated.last_progress_msg_id = None;
        cleared = true;
    }

    process_stream_events(bot, chat_id, thread_id, stream, &session_data, (config, cli, sessions), &mut state).await?;

    if let Some(mid) = state.pub_id
        && (state.last_text.contains("🛠️ **Tool Calls:") || state.last_text.contains("<!-- Waiting for")) {
            let tok = std::env::var("TELEGRAM_TOKEN").unwrap_or_else(|_| config.telegram_token.clone());
            super::typing::set_progress_reaction(mid.0, chat_id, tok);
            updated.last_progress_msg_id = Some(mid.0);
        }

    if state.last_sid.is_some() || cleared || updated.last_progress_msg_id.is_some() {
        persist_stream_session_update(sessions, &session_data, updated, state.last_sid, config).await;
    }
    Ok(())
}
