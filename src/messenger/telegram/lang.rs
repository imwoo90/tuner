//! # Chat Thread Locale Detector and Switcher
//!
//! ## Overview
//! Automatically determines and updates the internationalization language locale context for
//! specific Telegram chat threads based on configuration and commands.
//!
//! ## Search Tags
//! #lang

use teloxide::prelude::*;
use teloxide::types::Message;
use crate::config::CliConfig;
use crate::session::manager::SessionManager;


fn build_lang_keyboard() -> teloxide::types::InlineKeyboardMarkup {
    let keyboard: Vec<Vec<teloxide::types::InlineKeyboardButton>> = crate::i18n::LANGUAGES.iter().map(|&(code, name)| {
        vec![teloxide::types::InlineKeyboardButton::callback(
            format!("{} ({})", name, code),
            format!("lang:{}", code),
        )]
    }).collect();
    teloxide::types::InlineKeyboardMarkup::new(keyboard)
}

pub(crate) async fn handle_lang_command(
    bot: &teloxide::Bot,
    msg: &Message,
    args: &str,
    config: &CliConfig,
    sessions: &SessionManager,
) -> Result<(), teloxide::RequestError> {
    let topic_id = crate::telegram::get_topic_id(msg);
    let key = crate::session::key::SessionKey::telegram(msg.chat.id.0, topic_id);
    if args.is_empty() {
        let markup = build_lang_keyboard();
        let mut req = bot.send_message(msg.chat.id, crate::t!("bot.language_select_header"));
        if let Some(tid) = msg.thread_id {
            req = req.message_thread_id(tid);
        }
        if let Err(e) = req.reply_markup(markup).await {
            eprintln!("❌ [tuner] Failed to send language select keyboard: {:?}", e);
        }
    } else {
        let default_model = config.model.as_deref().unwrap_or("");
        let Ok((sess, _)) = sessions.get_or_init_session(&key, &config.provider, default_model).await else {
            return Ok(());
        };
        
        let target_lang = if crate::i18n::LANGUAGES.iter().any(|(code, _)| *code == args) {
            args
        } else {
            "en"
        };
        
        let _ = sessions
            .mutate_session(&key, Some(&sess), |cur| {
                cur.language = Some(target_lang.to_string());
                true
            })
            .await;
        
        let confirm = crate::i18n::t_for_lang(target_lang, "bot.language_switch_success", &[("language", target_lang)]);
        if let Err(e) = super::commands::send_reply(bot, msg, confirm).await {
            eprintln!("❌ [tuner] Failed to send language switch success message: {:?}", e);
        }
    }
    Ok(())
}

pub(crate) async fn send_effort_menu(
    bot: &Bot,
    msg: &Message,
    sess: &crate::session::data::SessionData,
    cfg_eff: Option<&str>,
    eff_model: &str,
) {
    let cur = sess
        .effort
        .as_deref()
        .or(cfg_eff)
        .filter(|s| !s.is_empty())
        .and_then(|e| super::reply_directives::clamp_session_effort(&sess.provider, eff_model, e))
        .unwrap_or_else(|| "default".to_string());
    let text = super::formatting::markdown_to_telegram_html(&format!(
        "Current reasoning effort is `{cur}`. Select a new reasoning effort level:"
    ));
    let mut req = bot.send_message(msg.chat.id, text).parse_mode(teloxide::types::ParseMode::Html);
    if let Some(tid) = msg.thread_id {
        req = req.message_thread_id(tid);
    }
    let _ = req.reply_markup(super::reply::build_effort_keyboard_for_session(&sess.provider, eff_model)).await;
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    #[tokio::test]
    async fn test_telegram_language_switching_across_yields() {
        crate::i18n::TASK_ACTIVE_LANG.scope("ko".to_string(), async {
            tokio::task::yield_now().await;
            assert_eq!(crate::i18n::get_language(), "ko");
            
            tokio::task::yield_now().await;
            assert_eq!(crate::i18n::get_language(), "ko");
            
            let report = crate::t!("bot.status", agy_status="ok", token_present="ok", session_count=1, provider="antigravity", model="opus");
            assert!(report.contains("상태 리포트") || report.contains("Status Report"));
        }).await;

        crate::i18n::TASK_ACTIVE_LANG.scope("en".to_string(), async {
            tokio::task::yield_now().await;
            assert_eq!(crate::i18n::get_language(), "en");
        }).await;
    }
}
