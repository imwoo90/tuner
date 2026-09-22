//! # Session Initializer and Workspace Provisioner
//!
//! ## Overview
//! Resolves target paths for new chat threads, triggers workspace setup rules (CLAUDE.md/GEMINI.md),
//! clones custom skills, and boots up CLI processes when active sessions are required.
//!
//! ## Collaboration Graph
//! - Called by [`super::handler::handle_message`] when resolving or creating sessions.
//! - Instantiates rules through [`RulesSelector`](crate::workspace::rules::RulesSelector).
//!
//! ## Search Tags
//! #session-initializer, #workspace-provisioner, #rules-sync

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
    initialize_session_with_prompt(bot, msg, sessions, sess, cli, config, None).await
}

pub(crate) async fn initialize_session_with_prompt(
    bot: &Bot,
    msg: &Message,
    sessions: &SessionManager,
    sess: &mut SessionData,
    cli: &AntigravityCli,
    config: &CliConfig,
    startup_prompt: Option<&str>,
) -> Result<String, teloxide::RequestError> {
    let provider = &config.provider;
    let session_id = sess.get_session_id(provider);
    if !session_id.is_empty() {
        if session_id.starts_with("mock-") || cli.is_session_alive(&session_id) {
            return Ok(session_id);
        }
        eprintln!("⚠️ [tuner] Session {} for topic {:?} is expired. Resetting...", session_id, sess.topic_name);
        sess.set_session_id(provider, "");
        let _ = sessions.preserve_session_identity(sess).await;
    }

    if cfg!(test) {
        let mock_sid = "mock-session-123".to_string();
        sess.set_session_id(provider, &mock_sid);
        let _ = sessions.preserve_session_identity(sess).await;
        return Ok(mock_sid);
    }

    let tok = std::env::var("TELEGRAM_TOKEN").unwrap_or_else(|_| config.telegram_token.clone());
    let _g = super::typing::TelegramTypingGuard::new(bot.clone(), tok, msg).await;
    boot_fresh_session(bot, msg, sessions, sess, cli, config, startup_prompt).await
}

async fn handle_boot_response(
    bot: &Bot,
    msg: &Message,
    sessions: &SessionManager,
    sess: &mut SessionData,
    provider: &str,
    send_res: Result<crate::cli::CliResponse, String>,
    config: &CliConfig,
) -> Result<String, teloxide::RequestError> {
    match send_res {
        Ok(res) => {
            if let Some(ref new_sid) = res.session_id {
                sess.set_session_id(provider, new_sid);
                let _ = sessions.preserve_session_identity(sess).await;

                let reply_text = if res.result.trim().is_empty() {
                    crate::t!("bot.new_session")
                } else {
                    res.result.clone()
                };

                let mut req = bot.send_message(msg.chat.id, reply_text.clone());
                if let Some(tid) = msg.thread_id {
                    req = req.message_thread_id(tid);
                }
                let sent = req.await;

                let sent_id = sent.as_ref().ok().map(|m| m.id.0);
                let is_ok = sent.is_ok();
                let err_str = sent.as_ref().err().map(|e| e.to_string());

                crate::messenger::telegram::history::log_telegram_message(
                    &config.working_dir,
                    new_sid,
                    sess.topic_id,
                    sess.topic_name.as_deref(),
                    "bot",
                    sent_id,
                    &reply_text,
                    is_ok,
                    err_str.as_deref(),
                );

                sent?;
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

async fn boot_fresh_session(
    bot: &Bot,
    msg: &Message,
    sessions: &SessionManager,
    sess: &mut SessionData,
    cli: &AntigravityCli,
    config: &CliConfig,
    startup_prompt: Option<&str>,
) -> Result<String, teloxide::RequestError> {
    let provider = &config.provider;
    let default_prompt = crate::t!("bot.session_init_prompt");
    let initial_prompt = startup_prompt.unwrap_or(&default_prompt);
    let ws = cli.agy_workspace();
    let mut session_cli = cli.clone();
    let (chat_id, topic_id) = (
        msg.chat.id.0,
        sess.topic_id.or_else(|| msg.thread_id.map(|t| t.0.0 as i64)),
    );
    session_cli.config.chat_id = chat_id;
    session_cli.config.topic_id = topic_id;

    let mut cancel_rx = cli.sessions.register_boot(chat_id, topic_id).await;
    let send_fut = session_cli.send(initial_prompt, None, false, ws);
    tokio::pin!(send_fut);

    let send_res = tokio::select! {
        res = &mut send_fut => {
            cli.sessions.unregister_boot(chat_id, topic_id).await;
            res
        }
        Ok(()) = &mut cancel_rx => {
            cli.sessions.unregister_boot(chat_id, topic_id).await;
            eprintln!("⚠️ [tuner] Session boot cancelled by user (/stop) in chat {}, topic {:?}", chat_id, topic_id);
            return Ok(String::new());
        }
    };

    handle_boot_response(bot, msg, sessions, sess, provider, send_res, config).await
}
