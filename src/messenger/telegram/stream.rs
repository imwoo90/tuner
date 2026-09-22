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
        let chunk_clone = chunk_str.clone();
        async move {
            if is_first {
                if let Some(mid) = msg_id {
                    let edit_res = bot.edit_message_text(chat_id, mid, chunk_clone.clone())
                        .parse_mode(teloxide::types::ParseMode::Html).await;
                    match edit_res {
                        Ok(edited) => return Ok(edited.id),
                        Err(e) if is_message_not_modified(&e) => return Ok(mid),
                        Err(e) => eprintln!("⚠️ [tuner] edit_message_text failed, fallback: {:?}", e),
                    }
                }
            }
            let mut req = bot.send_message(chat_id, chunk_clone).parse_mode(teloxide::types::ParseMode::Html);
            if let Some(tid) = thread_id {
                req = req.message_thread_id(teloxide::types::ThreadId(teloxide::types::MessageId(tid)));
            }
            req.await.map(|m| m.id)
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
    let mut last_session_id = None;
    if let Some(ref sid) = resp.session_id {
        last_session_id = Some(sid.clone());
    }
    if resp.stderr == "Interrupted by user (/stop)" {
        return Ok(last_session_id);
    }
    let raw_text = if resp.is_error {
        let code = resp.returncode.unwrap_or(1);
        let error_msg = if !resp.stderr.is_empty() { &resp.stderr } else { &resp.result };
        crate::cli::antigravity::error_parser::parse_cli_error(error_msg, code)
    } else {
        resp.result
    };

    let html_text = formatting::markdown_to_telegram_html(&raw_text);
    let chunks = formatting::split_html_message(&html_text, 4000);

    let (final_success, final_error, sent_msg_id) =
        send_chunks_to_telegram(bot, chat_id, msg_id, thread_id, &chunks).await;

    let _ = super::attachments::send_file_attachments(bot, chat_id, thread_id, &raw_text, config).await;

    if let Some(ref sid) = last_session_id {
        super::history::log_telegram_message(
            &config.working_dir,
            sid,
            topic_id,
            topic_name,
            "bot",
            sent_msg_id,
            &raw_text,
            final_success,
            final_error.as_deref(),
        );
    }
    Ok(last_session_id)
}




fn truncate_streaming_preview(delta: &str) -> String {
    if delta.chars().count() > 3900 {
        let tail: String = delta.chars().rev().take(3800).collect();
        let tail: String = tail.chars().rev().collect();
        format!("...[중간 생략]...\n{}", tail)
    } else {
        delta.to_string()
    }
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
    let preview = truncate_streaming_preview(delta);
    let limiter = super::rate_limiter::global_chat_rate_limiter();

    if let Some(mid) = *pub_msg_id {
        if last_edit.elapsed() >= Duration::from_secs(2) {
            let res = limiter.execute_rate_limited(chat_id, || {
                let d = preview.clone();
                async move { bot.edit_message_text(chat_id, mid, d).await }
            }).await;
            if let Err(e) = res {
                if !is_message_not_modified(&e) {
                    eprintln!("❌ [tuner] Failed to edit streaming message (chat: {}, msg: {}): {:?}", chat_id, mid, e);
                }
            }
            *last_edit = Instant::now();
        }
    } else {
        let res = limiter.execute_rate_limited(chat_id, || {
            let d = preview.clone();
            async move {
                let mut req = bot.send_message(chat_id, d);
                if let Some(tid) = thread_id { req = req.message_thread_id(teloxide::types::ThreadId(teloxide::types::MessageId(tid))); }
                req.await
            }
        }).await;
        match res {
            Ok(sent) => {
                *pub_msg_id = Some(sent.id);
                *last_edit = Instant::now();
            }
            Err(e) => {
                eprintln!("❌ [tuner] Failed to send streaming initial message (chat: {}, thread: {:?}): {:?}", chat_id, thread_id, e);
            }
        }
    }
    Ok(())
}

async fn process_stream_events(
    bot: &Bot,
    chat_id: ChatId,
    thread_id: Option<i32>,
    mut stream: futures::stream::BoxStream<'_, StreamEvent>,
    session_data: &crate::session::data::SessionData,
    config: &CliConfig,
    last_text: &mut String,
    pub_msg_id: &mut Option<teloxide::types::MessageId>,
    last_session_id: &mut Option<String>,
    cli: &crate::cli::antigravity::AntigravityCli,
) -> Result<(), teloxide::RequestError> {
    let mut last_edit = Instant::now();
    use futures::StreamExt;
    while let Some(event) = stream.next().await {
        match event {
            StreamEvent::TextDelta(delta) => {
                handle_text_delta(bot, chat_id, thread_id, &delta, last_text, pub_msg_id, &mut last_edit).await?;
            }
            StreamEvent::AskQuestion(ask) => {
                super::ask_process::handle_ask_question_event(bot, chat_id, thread_id, ask, session_data, config, cli).await?;
            }
            StreamEvent::Result(resp) => {
                *last_text = resp.result.clone();
                if let Ok(Some(sid)) = handle_stream_result(
                    bot,
                    chat_id,
                    *pub_msg_id,
                    thread_id,
                    session_data.topic_id,
                    session_data.topic_name.as_deref(),
                    resp,
                    config,
                ).await {
                    *last_session_id = Some(sid);
                }
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
    let mut last_session_id = None;
    let mut pub_msg_id = None;
    let mut last_text = String::new();

    let mut updated = session_data.clone();
    let mut cleared_old_reaction = false;
    if let Some(last_mid) = session_data.last_progress_msg_id {
        let tok = std::env::var("TELEGRAM_TOKEN").unwrap_or_else(|_| config.telegram_token.clone());
        super::typing::clear_old_progress_reaction(last_mid, chat_id, tok);
        updated.last_progress_msg_id = None;
        cleared_old_reaction = true;
    }

    process_stream_events(
        bot,
        chat_id,
        thread_id,
        stream,
        &session_data,
        config,
        &mut last_text,
        &mut pub_msg_id,
        &mut last_session_id,
        cli,
    ).await?;

    if let Some(mid) = pub_msg_id {
        if last_text.contains("🛠️ **Tool Calls:") || last_text.contains("<!-- Waiting for") {
            let tok = std::env::var("TELEGRAM_TOKEN").unwrap_or_else(|_| config.telegram_token.clone());
            super::typing::set_progress_reaction(mid.0, chat_id, tok);
            updated.last_progress_msg_id = Some(mid.0);
        }
    }

    if last_session_id.is_some() || cleared_old_reaction || updated.last_progress_msg_id.is_some() {
        if let Some(sid) = last_session_id {
            updated.set_session_id(&config.provider, &sid);
        }
        let _ = sessions.update_session(&updated, 0.0, 0).await;
    }
    Ok(())
}

