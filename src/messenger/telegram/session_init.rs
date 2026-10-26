//! # Session Initializer and Workspace Provisioner
//!
//! ## Overview
//! Resolves target paths for new chat threads, triggers workspace setup rules (`AGENTS.md`),
//! clones custom skills, and boots up CLI processes when active sessions are required.
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
    let thread_id = msg.thread_id.map(|t| t.0.0);
    match send_res {
        Ok(mut res) => {
            if let Some(ref new_sid) = res.session_id.clone() {
                sess.set_session_id(provider, new_sid);
                let _ = sessions.preserve_session_identity(sess).await;

                if res.result.trim().is_empty() && !res.is_error {
                    res.result = crate::t!("bot.new_session");
                }
                let _ = super::stream::handle_stream_result(
                    bot,
                    msg.chat.id,
                    None,
                    thread_id,
                    (sess.topic_id, sess.topic_name.as_deref()),
                    res,
                    config,
                ).await;
                Ok(new_sid.to_string())
            } else {
                eprintln!("Initialization oneshot did not return a session_id");
                Ok(String::new())
            }
        }
        Err(e) => {
            eprintln!("Initialization oneshot failed: {:?}", e);
            let err_msg = crate::cli::antigravity::error_parser::parse_cli_error(&e, 1);
            let html = super::formatting::markdown_to_telegram_html(&err_msg);
            let chunks = super::formatting::split_html_message(&html, 4000);
            let _ = super::stream::send_chunks_to_telegram(bot, msg.chat.id, None, thread_id, &chunks).await;
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
    if !sess.model.is_empty() && sess.model != "antigravity-default" {
        session_cli.config.model = Some(sess.model.clone());
    }
    if sess.effort.is_some() {
        session_cli.config.effort = sess.effort.clone();
    }

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
