//! # Telegram Callback Query Handlers
//!
//! This module handles inline keyboard callback queries, such as
//! model switching, language selection, cron execution, and interactive responses.

use teloxide::prelude::*;
use crate::config::CliConfig;
use crate::session::manager::SessionManager;
use crate::cron::manager::CronManager;
use crate::cli::antigravity::AntigravityCli;
use super::ask_callbacks::{
    handle_ask_answer_callback, handle_ask_submit_callback,
    handle_ask_write_callback, handle_ask_prev_callback
};
use super::multi_select::handle_ask_multi_callback;

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
                handle_ask_answer_callback(&bot, msg, d, &cli, &sessions, &config).await;
            } else if d.starts_with("ask_mul:") {
                handle_ask_multi_callback(&bot, msg, d, &cli.sessions).await;
            } else if d.starts_with("ask_sub:") {
                handle_ask_submit_callback(&bot, msg, d, &cli, &sessions, &config).await;
            } else if d.starts_with("ask_write:") {
                handle_ask_write_callback(&bot, msg, d, &cli).await;
            } else if d.starts_with("ask_prev:") {
                handle_ask_prev_callback(&bot, msg, d, &cli, &sessions, &config).await;
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
    handle_callback_query_inner(bot, q, config, sessions, cron, cli).await
}
