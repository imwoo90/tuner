//! # Telegram Callback Query Handlers
//!
//! This module handles inline keyboard callback queries, such as
//! model switching, language selection, cron execution, and interactive responses.

use teloxide::prelude::*;
use crate::config::CliConfig;
use crate::session::manager::SessionManager;
use crate::cron::manager::CronManager;
use crate::cli::antigravity::AntigravityCli;

async fn handle_model_callback(
    bot: &teloxide::Bot,
    msg: &Message,
    model: &str,
    sessions: &SessionManager,
    config: &CliConfig,
) {
    let key = crate::session::key::SessionKey::telegram(msg.chat.id.0, crate::telegram::get_topic_id(msg));
    let dm = config.model.as_deref().unwrap_or("antigravity-default");
    if let Ok((mut s, _)) = sessions.resolve_session(&key, &config.provider, dm).await {
        s.model = model.to_string();
        let _ = sessions.update_session(&s, 0.0, 0).await;
        let _ = bot.edit_message_text(msg.chat.id, msg.id, crate::t!("bot.model_switch_success", model = model)).await;
    }
}

async fn handle_lang_callback(
    bot: &teloxide::Bot,
    msg: &Message,
    lang: &str,
    sessions: &SessionManager,
    config: &CliConfig,
) {
    let key = crate::session::key::SessionKey::telegram(msg.chat.id.0, crate::telegram::get_topic_id(msg));
    let dm = config.model.as_deref().unwrap_or("antigravity-default");
    if let Ok((mut s, _)) = sessions.resolve_session(&key, &config.provider, dm).await {
        s.language = Some(lang.to_string());
        let _ = sessions.update_session(&s, 0.0, 0).await;
        crate::i18n::set_language(lang);
        let _ = bot.edit_message_text(msg.chat.id, msg.id, crate::t!("bot.language_switch_success", language = lang)).await;
    }
}

async fn update_telegram_ask_ui(
    bot: &teloxide::Bot,
    msg: &Message,
    index: usize,
    data: &str,
) {
    let mut chosen_text = format!("Option {}", index + 1);
    if let Some(reply_markup) = msg.reply_markup() {
        for row in &reply_markup.inline_keyboard {
            for button in row {
                if let teloxide::types::InlineKeyboardButtonKind::CallbackData(cbd) = &button.kind {
                    if cbd == data {
                        chosen_text = button.text.clone();
                    }
                }
            }
        }
    }
    let new_text = format!("{}\n\n(Selected: **{}**)", msg.text().unwrap_or(""), chosen_text);
    let html_text = crate::telegram::formatting::markdown_to_telegram_html(&new_text);
    let _ = bot.edit_message_text(msg.chat.id, msg.id, html_text)
        .parse_mode(teloxide::types::ParseMode::Html)
        .await;
}

async fn handle_ask_answer_callback(
    bot: &teloxide::Bot,
    msg: &Message,
    data: &str,
    cli: &AntigravityCli,
) {
    println!("🤖 [tuner] handle_ask_answer_callback: data = {}", data);
    let parts: Vec<&str> = data.split(':').collect();
    if parts.len() >= 3 {
        let session_id = parts[1];
        if let Ok(index) = parts[2].parse::<usize>() {
            let response_input = if index == 0 {
                "\r".to_string()
            } else {
                format!("{}\r", "j".repeat(index))
            };
            println!("🤖 [tuner] Writing to session: id = {}, input = {:?}", session_id, response_input);
            match cli.sessions.write_to_session(session_id, &response_input).await {
                Ok(written) => {
                    println!("🤖 [tuner] Write to session result: written = {}", written);
                    if written {
                        update_telegram_ask_ui(bot, msg, index, data).await;
                    }
                }
                Err(e) => {
                    eprintln!("❌ [tuner] Failed to write to session: {:?}", e);
                }
            }
        }
    }
}

async fn handle_callback_query_inner(
    bot: teloxide::Bot,
    q: teloxide::types::CallbackQuery,
    config: std::sync::Arc<CliConfig>,
    sessions: std::sync::Arc<SessionManager>,
    cron: std::sync::Arc<CronManager>,
    cli: std::sync::Arc<AntigravityCli>,
) -> Result<(), teloxide::RequestError> {
    use teloxide::prelude::*;
    if let Some(ref d) = q.data {
        if let Some(ref msg) = q.message {
            if let Some(m) = d.strip_prefix("model:") {
                handle_model_callback(&bot, msg, m, &sessions, &config).await;
            } else if let Some(m) = d.strip_prefix("lang:") {
                handle_lang_callback(&bot, msg, m, &sessions, &config).await;
            } else if d.starts_with("crn:") {
                let _ = crate::telegram::cron_selector::handle_cron_callback(&bot, msg.chat.id, msg.id, d, &cron).await;
            } else if d.starts_with("ask_ans:") {
                handle_ask_answer_callback(&bot, msg, d, &cli).await;
            }
        }
        let _ = bot.answer_callback_query(q.id).await;
    }
    Ok(())
}

pub(crate) async fn handle_callback_query(
    bot: teloxide::Bot,
    q: teloxide::types::CallbackQuery,
    config: std::sync::Arc<CliConfig>,
    sessions: std::sync::Arc<SessionManager>,
    cron: std::sync::Arc<CronManager>,
    cli: std::sync::Arc<AntigravityCli>,
) -> Result<(), teloxide::RequestError> {
    let mut active_lang = config.language.clone().unwrap_or_else(|| "en".to_string());
    if let Some(ref msg) = q.message {
        let key = crate::session::key::SessionKey::telegram(msg.chat.id.0, crate::telegram::get_topic_id(msg));
        let dm = config.model.as_deref().unwrap_or("antigravity-default");
        if let Ok((sess, _)) = sessions.resolve_session(&key, &config.provider, dm).await {
            active_lang = sess.language.unwrap_or_else(|| config.language.clone().unwrap_or_else(|| "en".to_string()));
        }
    }

    crate::i18n::TASK_ACTIVE_LANG.scope(active_lang, async move {
        handle_callback_query_inner(bot, q, config, sessions, cron, cli).await
    }).await
}
