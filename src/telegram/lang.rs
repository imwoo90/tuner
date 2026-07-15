use teloxide::prelude::*;
use teloxide::types::Message;
use crate::config::CliConfig;
use crate::session::manager::SessionManager;


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
        let mut keyboard = Vec::new();
        for &(code, name) in crate::i18n::LANGUAGES {
            keyboard.push(vec![teloxide::types::InlineKeyboardButton::callback(
                format!("{} ({})", name, code),
                format!("lang:{}", code),
            )]);
        }
        let markup = teloxide::types::InlineKeyboardMarkup::new(keyboard);
        let mut req = bot.send_message(msg.chat.id, crate::t!("bot.language_select_header"));
        if let Some(tid) = msg.thread_id {
            req = req.message_thread_id(tid);
        }
        let _ = req.reply_markup(markup).await;
    } else {
        let default_model = config.model.as_deref().unwrap_or("antigravity-default");
        let (mut sess, _) = sessions.resolve_session(&key, &config.provider, default_model).await.unwrap();
        
        let target_lang = if crate::i18n::LANGUAGES.iter().any(|(code, _)| *code == args) {
            args
        } else {
            "en"
        };
        
        sess.language = Some(target_lang.to_string());
        let _ = sessions.update_session(&sess, 0.0, 0).await;
        
        crate::i18n::set_language(target_lang);
        
        let mut req = bot.send_message(msg.chat.id, crate::t!("bot.language_switch_success", language = target_lang));
        if let Some(tid) = msg.thread_id {
            req = req.message_thread_id(tid);
        }
        let _ = req.await;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
