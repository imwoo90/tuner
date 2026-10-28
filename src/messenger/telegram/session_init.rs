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

async fn resolve_auto_handover(
    sess: &mut SessionData,
    config: &CliConfig,
    sessions: &SessionManager,
) -> Option<String> {
    let past_ids = sess.get_past_session_ids(&config.provider);
    if past_ids.is_empty() && sess.last_handover_summary.is_none() {
        return None;
    }
    let last_sid = past_ids.last().map(String::as_str).unwrap_or("");
    let handover = super::handover::build_topic_handover(
        &config.working_dir,
        last_sid,
        sess.topic_id,
        sess.topic_name.clone(),
        sess.last_handover_summary.as_deref(),
        &past_ids,
    )?;
    sess.last_handover_summary = Some(format!(
        "Objective: {}\nDecisions: {}\nProgress: {}",
        handover.topic_objective,
        handover.agreed_decisions.join("; "),
        handover.current_progress
    ));
    let _ = sessions.preserve_session_identity(sess).await;
    Some(handover.handover_prompt)
}

pub(crate) fn has_alive_session(
    sess: &SessionData,
    cli: &AntigravityCli,
    provider: &str,
) -> bool {
    let sid = sess.get_session_id(provider);
    !sid.is_empty() && (sid.starts_with("mock-") || cli.is_session_alive(&sid))
}

pub(crate) fn combine_handover_and_prompt(handover: &str, user_prompt: &str) -> String {
    let header = handover.split("\n---\n").next().unwrap_or(handover);
    format!(
        "{}\n---\nApply the restored topic context above and directly respond to the user's message below without a separate session greeting:\n\n{}",
        header, user_prompt
    )
}

pub(crate) async fn initialize_session_with_prompt(
    bot: &Bot,
    msg: &Message,
    sessions: &SessionManager,
    sess: &mut SessionData,
    cli: &AntigravityCli,
    config: &CliConfig,
    startup_prompt: Option<(&str, &str)>,
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

    let auto_handover = resolve_auto_handover(sess, config, sessions).await;
    let user_log_text = startup_prompt.map(|(u, _)| u);
    let raw_prompt = startup_prompt.map(|(_, p)| p);
    let combined;
    let effective_prompt = match (raw_prompt, auto_handover.as_deref()) {
        (Some(user_p), Some(ho)) => {
            combined = combine_handover_and_prompt(ho, user_p);
            Some(combined.as_str())
        }
        (Some(user_p), None) => Some(user_p),
        (None, ho) => ho,
    };

    if cfg!(test) {
        let mock_sid = "mock-session-123".to_string();
        sess.set_session_id(provider, &mock_sid);
        let _ = sessions.preserve_session_identity(sess).await;
        return Ok(mock_sid);
    }

    let tok = std::env::var("TELEGRAM_TOKEN").unwrap_or_else(|_| config.telegram_token.clone());
    let _g = super::typing::TelegramTypingGuard::new(bot.clone(), tok, msg).await;
    boot_fresh_session(bot, msg, sessions, sess, cli, (user_log_text, effective_prompt)).await
}

async fn handle_boot_success(
    bot: &Bot,
    msg: &Message,
    sessions: &SessionManager,
    sess: &mut SessionData,
    user_log_text: Option<&str>,
    mut res: crate::cli::CliResponse,
    config: &CliConfig,
) -> Result<String, teloxide::RequestError> {
    let thread_id = msg.thread_id.map(|t| t.0.0);
    if res.is_error {
        let topic = (sess.topic_id, sess.topic_name.as_deref());
        let _ = super::stream::handle_stream_result(bot, msg.chat.id, None, thread_id, topic, res, config).await;
        return Ok(String::new());
    }
    let Some(new_sid) = res.session_id.clone() else {
        eprintln!("Initialization oneshot did not return a session_id");
        return Ok(String::new());
    };
    sess.set_session_id(&config.provider, &new_sid);
    let _ = sessions.preserve_session_identity(sess).await;
    if res.result.trim().is_empty() {
        res.result = crate::t!("bot.new_session");
    }
    let topic = (sess.topic_id, sess.topic_name.as_deref());
    if let Some(user_txt) = user_log_text.filter(|s| !s.is_empty()) {
        super::history::log_telegram_message(&config.working_dir, &new_sid, topic, "user", Some(msg.id.0), user_txt, (true, None));
    }
    let _ = super::stream::handle_stream_result(bot, msg.chat.id, None, thread_id, topic, res, config).await;
    Ok(new_sid)
}

async fn handle_boot_response(
    bot: &Bot,
    msg: &Message,
    sessions: &SessionManager,
    sess: &mut SessionData,
    user_log_text: Option<&str>,
    send_res: Result<crate::cli::CliResponse, String>,
    config: &CliConfig,
) -> Result<String, teloxide::RequestError> {
    match send_res {
        Ok(res) => handle_boot_success(bot, msg, sessions, sess, user_log_text, res, config).await,
        Err(e) => {
            eprintln!("Initialization oneshot failed: {:?}", e);
            let err_msg = crate::cli::antigravity::error_parser::parse_cli_error(&e, 1, config.model.as_deref());
            let html = super::formatting::markdown_to_telegram_html(&err_msg);
            let chunks = super::formatting::split_html_message(&html, 4000);
            let _ = super::stream::send_chunks_to_telegram(bot, msg.chat.id, None, msg.thread_id.map(|t| t.0.0), &chunks).await;
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
    prompts: (Option<&str>, Option<&str>),
) -> Result<String, teloxide::RequestError> {
    let (user_log_text, startup_prompt) = prompts;
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

    handle_boot_response(bot, msg, sessions, sess, user_log_text, send_res, &session_cli.config).await
}
