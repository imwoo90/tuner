use teloxide::prelude::*;
use crate::config::CliConfig;
use crate::cli::StreamEvent;
use std::time::{Instant, Duration};
use super::formatting;

pub(crate) async fn handle_stream_result(
    bot: &Bot,
    chat_id: ChatId,
    msg_id: Option<teloxide::types::MessageId>,
    thread_id: Option<i32>,
    resp: crate::cli::CliResponse,
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

    let mut current_msg_id = msg_id;
    for (i, chunk) in chunks.iter().enumerate() {
        if let Some(mid) = current_msg_id {
            if i == 0 {
                let _ = bot.edit_message_text(chat_id, mid, chunk)
                    .parse_mode(teloxide::types::ParseMode::Html)
                    .await;
            } else {
                let mut req = bot.send_message(chat_id, chunk)
                    .parse_mode(teloxide::types::ParseMode::Html);
                if let Some(tid) = thread_id {
                    req = req.message_thread_id(tid);
                }
                let _ = req.await;
            }
        } else {
            let mut req = bot.send_message(chat_id, chunk)
                .parse_mode(teloxide::types::ParseMode::Html);
            if let Some(tid) = thread_id {
                req = req.message_thread_id(tid);
            }
            if let Ok(sent) = req.await {
                if i == 0 {
                    current_msg_id = Some(sent.id);
                }
            }
        }
    }
    Ok(last_session_id)
}
pub(crate) fn build_multi_select_keyboard(
    sess_id: &str,
    options: &[String],
    bitmap: &str,
) -> teloxide::types::InlineKeyboardMarkup {
    let mut keyboard = Vec::new();
    for (i, opt) in options.iter().enumerate() {
        let is_checked = bitmap.chars().nth(i).unwrap_or('0') == '1';
        let prefix = if is_checked { "✅ " } else { "⬜ " };
        let button_text = format!("{}{}", prefix, opt);
        let callback_data = format!("ask_mul:{}:{}:{}", sess_id, i, bitmap);
        keyboard.push(vec![teloxide::types::InlineKeyboardButton::callback(button_text, callback_data)]);
    }
    let submit_callback = format!("ask_sub:{}:{}", sess_id, bitmap);
    keyboard.push(vec![teloxide::types::InlineKeyboardButton::callback("완료 (Submit)", submit_callback)]);
    teloxide::types::InlineKeyboardMarkup::new(keyboard)
}

async fn handle_stream_ask_question(
    bot: &Bot,
    chat_id: ChatId,
    thread_id: Option<i32>,
    ask: crate::cli::AskQuestionData,
    session_data: &crate::session::data::SessionData,
    config: &CliConfig,
) -> Result<(), teloxide::RequestError> {
    let sess_id = session_data.get_session_id(&config.provider);
    let markup = if ask.is_multi_select {
        let initial_bitmap = "0".repeat(ask.options.len());
        build_multi_select_keyboard(&sess_id, &ask.options, &initial_bitmap)
    } else {
        let mut keyboard = Vec::new();
        for (i, opt) in ask.options.iter().enumerate() {
            let callback_data = format!("ask_ans:{}:{}", sess_id, i);
            keyboard.push(vec![teloxide::types::InlineKeyboardButton::callback(opt, callback_data)]);
        }
        teloxide::types::InlineKeyboardMarkup::new(keyboard)
    };
    let html_question = formatting::markdown_to_telegram_html(&ask.question);
    let mut req = bot.send_message(chat_id, html_question)
        .parse_mode(teloxide::types::ParseMode::Html)
        .reply_markup(markup);
    if let Some(tid) = thread_id {
        req = req.message_thread_id(tid);
    }
    let _ = req.await;
    Ok(())
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
                let sess_id = session_data.get_session_id(&config.provider);
                cli.sessions.set_ask_options(&sess_id, ask.options.clone()).await;
                cli.sessions.set_ask_active(&sess_id, true).await;
                let _ = handle_stream_ask_question(bot, chat_id, thread_id, ask, session_data, config).await;
            }
            StreamEvent::Result(resp) => {
                *last_text = resp.result.clone();
                if let Ok(Some(sid)) = handle_stream_result(bot, chat_id, *pub_msg_id, thread_id, resp).await {
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
