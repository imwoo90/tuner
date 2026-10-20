//! # Streaming Output Consumer for Telegram Chat
//!
//! ## Overview
//! Consumes character/event streams from active agent CLI PTY processes. Debounces text chunks
//! to minimize Telegram message update rate-limiting and renders real-time outputs.
//!
//! ## Collaboration Graph
//! - Receives stdout streams from [`AntigravityCli`](crate::cli::antigravity::AntigravityCli).
//! - Edits active Telegram messages dynamically using Teloxide.
//! - Feeds final execution status updates back to the session manager.
//!
//! ## Search Tags
//! #streaming-consumer, #debouncer, #rate-limiting, #message-updates

use teloxide::prelude::*;
use crate::config::CliConfig;
use crate::cli::StreamEvent;
use std::time::{Instant, Duration};
use super::formatting;


pub(crate) fn is_message_not_modified(err: &teloxide::RequestError) -> bool {
    match err {
        teloxide::RequestError::Api(teloxide::ApiError::MessageNotModified) => true,
        other => other.to_string().to_lowercase().contains("message is not modified"),
    }
}

async fn send_single_chunk(
    bot: &Bot,
    chat_id: ChatId,
    msg_id: Option<teloxide::types::MessageId>,
    thread_id: Option<i32>,
    chunk: &str,
    is_first: bool,
) -> Result<teloxide::types::MessageId, teloxide::RequestError> {
    let limiter = super::rate_limiter::global_chat_rate_limiter();
    let chunk_str = chunk.to_string();
    limiter.execute_rate_limited(chat_id, || {
        let c = chunk_str.clone();
        async move {
            let html = teloxide::types::ParseMode::Html;
            if let (true, Some(mid)) = (is_first, msg_id) {
                match bot.edit_message_text(chat_id, mid, c.clone()).parse_mode(html).await {
                    Ok(ed) => return Ok(ed.id),
                    Err(e) if is_message_not_modified(&e) => return Ok(mid),
                    Err(e) => if e.to_string().contains("can't parse entities") {
                        if let Ok(ed) = bot.edit_message_text(chat_id, mid, c.clone()).await { return Ok(ed.id); }
                    } else { eprintln!("⚠️ [tuner] edit_message_text fallback: {:?}", e); },
                }
            }
            let tid = thread_id.map(|t| teloxide::types::ThreadId(teloxide::types::MessageId(t)));
            let mut req = bot.send_message(chat_id, c.clone()).parse_mode(html);
            if let Some(t) = tid { req = req.message_thread_id(t); }
            match req.await {
                Ok(m) => Ok(m.id),
                Err(e) if e.to_string().contains("can't parse entities") => {
                    let mut r = bot.send_message(chat_id, c);
                    if let Some(t) = tid { r = r.message_thread_id(t); }
                    r.await.map(|m| m.id)
                }
                Err(e) => Err(e),
            }
        }
    }).await
}

pub(crate) async fn send_chunks_to_telegram(
    bot: &Bot,
    chat_id: ChatId,
    msg_id: Option<teloxide::types::MessageId>,
    thread_id: Option<i32>,
    chunks: &[String],
) -> (bool, Option<String>, Option<i32>) {
    let (mut cur_id, mut ok, mut err, mut sent_id) = (msg_id, true, None, None);
    for (i, chunk) in chunks.iter().enumerate() {
        match send_single_chunk(bot, chat_id, cur_id, thread_id, chunk, i == 0).await {
            Ok(mid) => {
                if cur_id.is_none() && i == 0 { cur_id = Some(mid); }
                sent_id = Some(mid.0);
            }
            Err(e) => {
                eprintln!("❌ [tuner] Stream chunk {}/{} failed: {:?}", i + 1, chunks.len(), e);
                ok = false;
                err = Some(e.to_string());
            }
        }
    }
    (ok, err, sent_id)
}

pub(crate) async fn handle_stream_result(
    bot: &Bot,
    chat_id: ChatId,
    msg_id: Option<teloxide::types::MessageId>,
    thread_id: Option<i32>,
    topic_id: Option<i64>,
    topic_name: Option<&str>,
    resp: crate::cli::CliResponse,
    config: &CliConfig,
) -> Result<Option<String>, teloxide::RequestError> {
    let last_sid = resp.session_id.clone();
    if resp.stderr == "Interrupted by user (/stop)" {
        return Ok(last_sid);
    }
    let raw_text = if resp.is_error {
        let err_msg = if !resp.stderr.is_empty() { &resp.stderr } else { &resp.result };
        crate::cli::antigravity::error_parser::parse_cli_error(err_msg, resp.returncode.unwrap_or(1))
    } else {
        resp.result
    };
    let chunks = formatting::split_html_message(&formatting::markdown_to_telegram_html(&raw_text), 4000);
    let (ok, err, sent_id) = send_chunks_to_telegram(bot, chat_id, msg_id, thread_id, &chunks).await;
    let _ = super::attachments::send_file_attachments(bot, chat_id, thread_id, &raw_text, config).await;
    if let Some(ref sid) = last_sid {
        super::history::log_telegram_message(&config.working_dir, sid, topic_id, topic_name, "bot", sent_id, &raw_text, ok, err.as_deref());
    }
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
    let preview = truncate_streaming_preview(delta);
    let full_html = formatting::markdown_to_telegram_html(&preview);
    formatting::split_html_message(&full_html, 3800)
        .into_iter()
        .last()
        .unwrap_or(full_html)
}

pub(crate) async fn handle_text_delta(
    bot: &Bot,
    chat_id: ChatId,
    thread_id: Option<i32>,
    delta: &str,
    last_text: &mut String,
    pub_msg_id: &mut Option<teloxide::types::MessageId>,
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

async fn process_stream_events(
    bot: &Bot,
    chat_id: ChatId,
    tid: Option<i32>,
    mut stream: futures::stream::BoxStream<'_, StreamEvent>,
    sdata: &crate::session::data::SessionData,
    cfg: &CliConfig,
    last_text: &mut String,
    pub_id: &mut Option<teloxide::types::MessageId>,
    last_sid: &mut Option<String>,
    cli: &crate::cli::antigravity::AntigravityCli,
) -> Result<(), teloxide::RequestError> {
    let (mut last_edit, mut pending) = (Instant::now(), false);
    let mut tick = tokio::time::interval(Duration::from_millis(700));
    use futures::StreamExt;
    loop {
        tokio::select! {
            ev = stream.next() => match ev {
                Some(StreamEvent::TextDelta(d)) => {
                    let prev = last_edit;
                    handle_text_delta(bot, chat_id, tid, &d, last_text, pub_id, &mut last_edit).await?;
                    pending = pub_id.is_some() && last_edit == prev;
                }
                Some(StreamEvent::AskQuestion(ask)) => super::ask_process::handle_ask_question_event(bot, chat_id, tid, ask, sdata, cfg, cli).await?,
                Some(StreamEvent::Result(resp)) => {
                    pending = false;
                    if !resp.result.is_empty() { *last_text = resp.result.clone(); }
                    if let Ok(Some(sid)) = handle_stream_result(bot, chat_id, *pub_id, tid, sdata.topic_id, sdata.topic_name.as_deref(), resp, cfg).await {
                        *last_sid = Some(sid);
                    }
                }
                None => break,
            },
            _ = tick.tick(), if pending => if last_edit.elapsed() >= Duration::from_secs(2) {
                let cur = last_text.clone();
                handle_text_delta(bot, chat_id, tid, &cur, last_text, pub_id, &mut last_edit).await?;
                pending = false;
            }
        }
    }
    Ok(())
}

pub(crate) async fn consume_stream(
    bot: &Bot,
    chat_id: ChatId,
    thread_id: Option<i32>,
    stream: futures::stream::BoxStream<'_, StreamEvent>,
    sessions: &crate::session::manager::SessionManager,
    session_data: crate::session::data::SessionData,
    config: &CliConfig,
    cli: &crate::cli::antigravity::AntigravityCli,
) -> Result<(), teloxide::RequestError> {
    let (mut last_sid, mut pub_msg_id, mut last_text) = (None, None, String::new());
    let mut updated = session_data.clone();
    let mut cleared = false;
    if let Some(last_mid) = session_data.last_progress_msg_id {
        let tok = std::env::var("TELEGRAM_TOKEN").unwrap_or_else(|_| config.telegram_token.clone());
        super::typing::clear_old_progress_reaction(last_mid, chat_id, tok);
        updated.last_progress_msg_id = None;
        cleared = true;
    }

    process_stream_events(bot, chat_id, thread_id, stream, &session_data, config, &mut last_text, &mut pub_msg_id, &mut last_sid, cli).await?;

    if let Some(mid) = pub_msg_id {
        if last_text.contains("🛠️ **Tool Calls:") || last_text.contains("<!-- Waiting for") {
            let tok = std::env::var("TELEGRAM_TOKEN").unwrap_or_else(|_| config.telegram_token.clone());
            super::typing::set_progress_reaction(mid.0, chat_id, tok);
            updated.last_progress_msg_id = Some(mid.0);
        }
    }

    if last_sid.is_some() || cleared || updated.last_progress_msg_id.is_some() {
        if let Some(sid) = last_sid { updated.set_session_id(&config.provider, &sid); }
        let _ = sessions.update_session(&updated, 0.0, 0).await;
    }
    Ok(())
}

