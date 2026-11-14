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

pub(crate) async fn ensure_session_ready_for_turn(
    bot: &Bot,
    msg: &Message,
    sessions: &std::sync::Arc<SessionManager>,
    sess: &mut SessionData,
    cli: &AntigravityCli,
    config: &CliConfig,
) -> Result<String, teloxide::RequestError> {
    let boot_prov = SessionData::effective_provider(&sess.provider, &config.provider);
    let mut sid = initialize_session_if_needed(bot, msg, sessions, sess, cli, config).await?;
    if sid.is_empty() {
        return Ok(String::new());
    }
    let mut cur_prov = SessionData::effective_provider(&sess.provider, &config.provider);
    if cur_prov != boot_prov {
        sid = initialize_session_if_needed(bot, msg, sessions, sess, cli, config).await?;
        if sid.is_empty() {
            return Ok(String::new());
        }
        cur_prov = SessionData::effective_provider(&sess.provider, &config.provider);
    }
    if cur_prov == "antigravity" {
        super::async_observer::spawn_session_async_observer(
            bot.clone(), msg, sid.clone(), cli.clone(), sessions.clone(), config.clone(),
        );
    }
    Ok(sid)
}

async fn mutate_or_local<F>(sessions: &SessionManager, sess: &mut SessionData, f: F)
where
    F: FnOnce(&mut SessionData) -> bool + Clone,
{
    if let Ok(Some(updated)) = sessions.mutate_session(&sess.session_key(), Some(sess), f.clone()).await {
        *sess = updated;
    } else {
        let _ = f(sess);
    }
}

async fn resolve_auto_handover(
    sess: &mut SessionData,
    config: &CliConfig,
    sessions: &SessionManager,
) -> Option<String> {
    let provider = SessionData::effective_provider(&sess.provider, &config.provider);
    let past_ids = sess.get_past_session_ids(provider);
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
    let summary = format!(
        "Objective: {}\nDecisions: {}\nProgress: {}",
        handover.topic_objective,
        handover.agreed_decisions.join("; "),
        handover.current_progress
    );
    mutate_or_local(sessions, sess, |cur| {
        cur.last_handover_summary = Some(summary.clone());
        true
    }).await;
    Some(handover.handover_prompt)
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
    let provider = SessionData::effective_provider(&sess.provider, &config.provider);
    let session_id = sess.get_session_id(provider);
    if !session_id.is_empty() {
        let mut check_cli = cli.clone();
        check_cli.config.provider = provider.to_string();
        if session_id.starts_with("mock-") || check_cli.is_session_alive(&session_id) {
            return Ok(session_id);
        }
        eprintln!("⚠️ [tuner] Session {} for topic {:?} is expired. Resetting...", session_id, sess.topic_name);
        mutate_or_local(sessions, sess, |cur| {
            if cur.get_session_id(provider) == session_id {
                cur.set_session_id(provider, "");
                return true;
            }
            false
        }).await;
    }

    let auto_handover = if startup_prompt.is_none() {
        resolve_auto_handover(sess, config, sessions).await
    } else {
        None
    };
    let effective_prompt = startup_prompt.or(auto_handover.as_deref());

    if cfg!(test) {
        let mock_sid = "mock-session-123".to_string();
        mutate_or_local(sessions, sess, |cur| {
            cur.set_session_id(provider, &mock_sid);
            cur.last_active = chrono::Utc::now().to_rfc3339();
            true
        }).await;
        return Ok(mock_sid);
    }

    let tok = std::env::var("TELEGRAM_TOKEN").unwrap_or_else(|_| config.telegram_token.clone());
    let _g = super::typing::TelegramTypingGuard::new(bot.clone(), tok, msg).await;
    boot_fresh_session(bot, msg, sessions, sess, cli, effective_prompt).await
}

#[cfg(test)]
pub(crate) fn merge_booted_session_id(
    sess: &mut SessionData,
    latest: Option<SessionData>,
    boot_prov: &str,
    new_sid: &str,
) {
    if let Some(mut cur) = latest {
        if cur.get_session_id(boot_prov).is_empty() {
            cur.set_session_id(boot_prov, new_sid);
        }
        *sess = cur;
    } else {
        sess.set_session_id(boot_prov, new_sid);
    }
}

async fn handle_boot_success(
    bot: &Bot,
    msg: &Message,
    sessions: &SessionManager,
    sess: &mut SessionData,
    mut res: crate::cli::CliResponse,
    config: &CliConfig,
) -> Result<String, teloxide::RequestError> {
    let thread_id = sess.topic_id.map(|t| t as i32).or_else(|| msg.thread_id.map(|t| t.0.0));
    if res.is_error {
        let topic = (sess.topic_id, sess.topic_name.as_deref());
        let _ = super::stream::handle_stream_result(bot, msg.chat.id, None, thread_id, topic, res, config).await;
        return Ok(String::new());
    }
    let Some(new_sid) = res.session_id.clone() else {
        eprintln!("Initialization oneshot did not return a session_id");
        return Ok(String::new());
    };
    let boot_prov = SessionData::effective_provider(&config.provider, &sess.provider);
    mutate_or_local(sessions, sess, |cur| {
        if cur.get_session_id(boot_prov).is_empty() {
            cur.set_session_id(boot_prov, &new_sid);
            cur.last_active = chrono::Utc::now().to_rfc3339();
            return true;
        }
        false
    }).await;
    if res.result.trim().is_empty() {
        res.result = crate::t!("bot.new_session");
    }
    let topic = (sess.topic_id, sess.topic_name.as_deref());
    let _ = super::stream::handle_stream_result(bot, msg.chat.id, None, thread_id, topic, res, config).await;
    Ok(new_sid)
}

async fn handle_boot_response(
    bot: &Bot,
    msg: &Message,
    sessions: &SessionManager,
    sess: &mut SessionData,
    send_res: Result<crate::cli::CliResponse, String>,
    config: &CliConfig,
) -> Result<String, teloxide::RequestError> {
    match send_res {
        Ok(res) => handle_boot_success(bot, msg, sessions, sess, res, config).await,
        Err(e) => {
            eprintln!("Initialization oneshot failed: {:?}", e);
            let err_msg = crate::cli::antigravity::error_parser::parse_cli_error(&e, 1, config.model.as_deref());
            let html = super::formatting::markdown_to_telegram_html(&err_msg);
            let chunks = super::formatting::split_html_message(&html, 4000);
            let thread_id = sess.topic_id.map(|t| t as i32).or_else(|| msg.thread_id.map(|t| t.0.0));
            let _ = super::stream::send_chunks_to_telegram(bot, msg.chat.id, None, thread_id, &chunks).await;
            Ok(String::new())
        }
    }
}

pub(crate) async fn configure_session_cli(
    cli: &AntigravityCli,
    sess: &SessionData,
    chat_id: i64,
    topic_id: Option<i64>,
) -> AntigravityCli {
    let mut session_cli = cli.clone();
    session_cli.config.chat_id = chat_id;
    session_cli.config.topic_id = topic_id;
    let prov = SessionData::effective_provider(&sess.provider, &session_cli.config.provider);
    if !matches!(prov, "claude" | "codex") {
        crate::cli::antigravity::model_spec::ensure_live_catalog_fresh().await;
    }
    sess.apply_to_cli_config(&mut session_cli.config);
    session_cli
}

async fn boot_fresh_session(
    bot: &Bot,
    msg: &Message,
    sessions: &SessionManager,
    sess: &mut SessionData,
    cli: &AntigravityCli,
    startup_prompt: Option<&str>,
) -> Result<String, teloxide::RequestError> {
    let default_prompt = crate::t!("bot.session_init_prompt");
    let initial_prompt = startup_prompt.unwrap_or(&default_prompt);
    let ws = cli.agy_workspace();
    let (chat_id, topic_id) = (
        msg.chat.id.0,
        sess.topic_id.or_else(|| super::get_topic_id(msg)),
    );
    let _turn_guard = cli.begin_topic_turn(chat_id, topic_id);
    let session_cli = configure_session_cli(cli, sess, chat_id, topic_id).await;

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

    handle_boot_response(bot, msg, sessions, sess, send_res, &session_cli.config).await
}
