use teloxide::prelude::*;
use crate::config::CliConfig;
use crate::cli::StreamEvent;
use std::time::{Instant, Duration};
use super::formatting;


async fn send_single_chunk(
    bot: &Bot,
    chat_id: ChatId,
    msg_id: Option<teloxide::types::MessageId>,
    thread_id: Option<i32>,
    chunk: &str,
    is_first: bool,
) -> Result<teloxide::types::Message, teloxide::RequestError> {
    if let Some(mid) = msg_id {
        if is_first {
            bot.edit_message_text(chat_id, mid, chunk)
                .parse_mode(teloxide::types::ParseMode::Html)
                .await
        } else {
            let mut req = bot.send_message(chat_id, chunk)
                .parse_mode(teloxide::types::ParseMode::Html);
            if let Some(tid) = thread_id { req = req.message_thread_id(tid); }
            req.await
        }
    } else {
        let mut req = bot.send_message(chat_id, chunk)
            .parse_mode(teloxide::types::ParseMode::Html);
        if let Some(tid) = thread_id { req = req.message_thread_id(tid); }
        req.await
    }
}

async fn send_chunks_to_telegram(
    bot: &Bot,
    chat_id: ChatId,
    msg_id: Option<teloxide::types::MessageId>,
    thread_id: Option<i32>,
    chunks: &[String],
) -> (bool, Option<String>, Option<i32>) {
    let mut current_msg_id = msg_id;
    let mut final_success = true;
    let mut final_error = None;
    let mut sent_msg_id = None;

    for (i, chunk) in chunks.iter().enumerate() {
        match send_single_chunk(bot, chat_id, current_msg_id, thread_id, chunk, i == 0).await {
            Ok(sent) => {
                if current_msg_id.is_none() && i == 0 {
                    current_msg_id = Some(sent.id);
                }
                sent_msg_id = Some(sent.id.0);
            }
            Err(e) => {
                final_success = false;
                final_error = Some(e.to_string());
            }
        }
    }
    (final_success, final_error, sent_msg_id)
}

pub(crate) async fn handle_stream_result(
    bot: &Bot,
    chat_id: ChatId,
    msg_id: Option<teloxide::types::MessageId>,
    thread_id: Option<i32>,
    resp: crate::cli::CliResponse,
    config: &CliConfig,
) -> Result<Option<String>, teloxide::RequestError> {
    let mut last_session_id = None;
    if let Some(ref sid) = resp.session_id {
        last_session_id = Some(sid.clone());
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

    if let Some(ref sid) = last_session_id {
        super::history::log_telegram_message(
            &config.working_dir,
            sid,
            "bot",
            sent_msg_id,
            &raw_text,
            final_success,
            final_error.as_deref(),
        );
    }
    Ok(last_session_id)
}


fn clear_old_progress_reaction(last_mid: i32, chat_id: ChatId, tok: String) {
    tokio::spawn(async move {
        let client = reqwest::Client::new();
        let url = format!("https://api.telegram.org/bot{}/setMessageReaction", tok);
        let body = serde_json::json!({
            "chat_id": chat_id.0,
            "message_id": last_mid,
            "reaction": []
        });
        let _ = client.post(&url).json(&body).send().await;
    });
}

fn set_progress_reaction(msg_id_val: i32, chat_id: ChatId, tok: String) {
    tokio::spawn(async move {
        let client = reqwest::Client::new();
        let url = format!("https://api.telegram.org/bot{}/setMessageReaction", tok);
        let body = serde_json::json!({
            "chat_id": chat_id.0,
            "message_id": msg_id_val,
            "reaction": [
                {
                    "type": "emoji",
                    "emoji": "⏳"
                }
            ]
        });
        let _ = client.post(&url).json(&body).send().await;
    });
}

async fn handle_text_delta(
    bot: &Bot,
    chat_id: ChatId,
    thread_id: Option<i32>,
    delta: &str,
    last_text: &mut String,
    pub_msg_id: &mut Option<teloxide::types::MessageId>,
    last_edit: &mut Instant,
) -> Result<(), teloxide::RequestError> {
    *last_text = delta.to_string();
    if let Some(mid) = *pub_msg_id {
        if last_edit.elapsed() >= Duration::from_secs(2) {
            let _ = bot.edit_message_text(chat_id, mid, delta).await;
            *last_edit = Instant::now();
        }
    } else {
        let mut req = bot.send_message(chat_id, delta);
        if let Some(tid) = thread_id { req = req.message_thread_id(tid); }
        if let Ok(sent) = req.await {
            *pub_msg_id = Some(sent.id);
            *last_edit = Instant::now();
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
                if let Ok(Some(sid)) = handle_stream_result(bot, chat_id, *pub_msg_id, thread_id, resp, config).await {
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
        clear_old_progress_reaction(last_mid, chat_id, tok);
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
        if last_text.contains("**[Ductor Background Progress]**") || last_text.contains("<!-- Waiting for") {
            let tok = std::env::var("TELEGRAM_TOKEN").unwrap_or_else(|_| config.telegram_token.clone());
            set_progress_reaction(mid.0, chat_id, tok);
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

