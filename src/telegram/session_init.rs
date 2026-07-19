use teloxide::prelude::*;
use crate::cli::antigravity::AntigravityCli;
use crate::cli::AgentProvider;
use crate::session::manager::SessionManager;
use crate::session::data::SessionData;
use crate::config::CliConfig;

pub(crate) async fn initialize_session_if_needed(
    bot: &Bot,
    msg: &Message,
    sessions: &SessionManager,
    sess: &mut SessionData,
    cli: &AntigravityCli,
    config: &CliConfig,
) -> Result<String, teloxide::RequestError> {
    let provider = &config.provider;
    let session_id = sess.get_session_id(provider);
    if !session_id.is_empty() {
        return Ok(session_id);
    }

    if cfg!(test) {
        let mock_sid = "mock-session-123".to_string();
        sess.set_session_id(provider, &mock_sid);
        let _ = sessions.preserve_session_identity(sess).await;
        return Ok(mock_sid);
    }

    let tok = std::env::var("TELEGRAM_TOKEN").unwrap_or_else(|_| config.telegram_token.clone());
    let _g = super::typing::TelegramTypingGuard::new(bot.clone(), tok, msg).await;

    let startup_prompt = crate::t!("bot.session_init_prompt");

    let ws = cli.agy_workspace();
    match cli.send(&startup_prompt, None, false, ws).await {
        Ok(res) => {
            if let Some(ref new_sid) = res.session_id {
                sess.set_session_id(provider, new_sid);
                let _ = sessions.preserve_session_identity(sess).await;

                let reply_text = if res.result.trim().is_empty() {
                    crate::t!("bot.new_session")
                } else {
                    res.result.clone()
                };

                let mut req = bot.send_message(msg.chat.id, reply_text);
                if let Some(tid) = msg.thread_id {
                    req = req.message_thread_id(tid);
                }
                let _ = req.await?;

                Ok(new_sid.to_string())
            } else {
                eprintln!("Initialization oneshot did not return a session_id");
                Ok(String::new())
            }
        }
        Err(e) => {
            eprintln!("Initialization oneshot failed: {:?}", e);
            Ok(String::new())
        }
    }
}
