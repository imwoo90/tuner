//! # Telegram Bot Interface Module (index.md)
//!
//! ## Overview
//! Handles ingress message updates, routes commands, coordinates PTY streaming output delivery,
//! and manages UI elements (inline keyboards, callbacks).
//!
//! ## Module Components
//! - [`commands`]: Handles slash commands (`/start`, `/model`, `/new`, `/abort`).
//! - [`callbacks`]: Handles interactive menu callback buttons clicks.
//! - [`stream`]: Debounces and streams raw stdout/stderr from CLI PTYs to Telegram.
//! - [`reply`]: Standardizes media downloads and prepends reply histories to prompt streams.
//! - [`transport`]: Adapts internal message envelopes to Teloxide API calls.
//! - [`attachments`]: Downloads files/photos and places them in the workspace.
//! - [`topic_cache`]: Maps topic names to IDs for topic auto-creation.
//!
//! ## Search Tags
//! #telegram-bot, #message-routing, #pty-streaming, #inline-keyboards, #chat-commands

use teloxide::prelude::*;
use crate::config::CliConfig;
use crate::cli::{antigravity::AntigravityCli, AgentProvider};
use crate::session::{manager::SessionManager, data::SessionData};
use std::sync::Arc;

pub mod formatting;
#[cfg(test)]
pub mod formatting_tests;
pub mod reply;
pub mod session_init;
pub mod history;
#[cfg(test)]
pub mod reply_tests;
#[cfg(test)]
pub mod handler_tests;
#[cfg(test)]
pub mod media_tests;
#[cfg(test)]
pub mod forum_tests;
#[cfg(test)]
pub mod ask_abort_tests;
#[cfg(test)]
pub mod test_helpers;
#[cfg(test)]
pub static TEST_ENV_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());
pub mod commands;
pub mod cron_selector;
pub mod topic_cache;
pub mod stream;
pub mod transport;
pub mod lang;
pub mod callbacks;
pub mod ask_callbacks;
pub mod ask_helpers;
pub mod ask_process;
pub mod multi_select;
pub mod runner;
pub mod attachments;
pub mod media_group;
pub mod upgrade;

pub(crate) use reply::{build_reply_prompt, parse_model_directive};
pub use transport::TelegramTransport;
pub mod typing;


pub use reply::get_topic_id;

pub use topic_cache::{BotInfo, TopicNameCache};

pub(crate) async fn run_cli_stream(
    bot: &Bot,
    msg: &Message,
    prompt: &str,
    sid: &str,
    cli: &AntigravityCli,
    sessions: &SessionManager,
    sess: SessionData,
    config: &CliConfig,
) -> Result<(), teloxide::RequestError> {
    let tok = std::env::var("TELEGRAM_TOKEN").unwrap_or_else(|_| config.telegram_token.clone());
    let _g = typing::TelegramTypingGuard::new(bot.clone(), tok, msg).await;
    let mut cc = cli.clone();
    cc.config.chat_id = msg.chat.id.0;
    cc.config.topic_id = msg.thread_id.map(|t| t as i64);
    if !sess.model.is_empty() { cc.config.model = Some(sess.model.clone()); }
    match cc.send_streaming(prompt, (!sid.is_empty()).then_some(sid), false, config.working_dir.clone()).await {
        Ok(s) => stream::consume_stream(bot, msg.chat.id, msg.thread_id, s, sessions, sess, config, cli).await?,
        Err(e) => {
            eprintln!("CLI ERROR: {:?}", e);
            let mut r = bot.send_message(msg.chat.id, format!("❌ Error: {}", e));
            if let Some(t) = msg.thread_id { r = r.message_thread_id(t); }
            let _ = r.await;
        }
    }
    Ok(())
}

async fn handle_model_override(
    bot: &Bot,
    msg: &Message,
    mo: &str,
    sess: &mut crate::session::data::SessionData,
    sessions: &SessionManager,
    empty: bool,
) -> Result<bool, teloxide::RequestError> {
    sess.model = mo.to_string();
    let _ = sessions.update_session(sess, 0.0, 0).await;
    if empty {
        let mut r = bot.send_message(msg.chat.id, format!("Next message will use {}", mo));
        if let Some(t) = msg.thread_id { r = r.message_thread_id(t); }
        let _ = r.await;
        return Ok(true);
    }
    Ok(false)
}



fn inject_pre_downloaded_files(prompt: &mut String, files: &[String]) {
    if !files.is_empty() {
        let mut media_hint = String::new();
        if files.len() == 1 {
            media_hint = format!(
                "[SYSTEM HINT] The user attached a file. You can read/view it by calling `view_file` at path: `{}`\n\n",
                files[0]
            );
        } else {
            media_hint.push_str("[SYSTEM HINT] The user attached multiple files. You can view them at:\n");
            for f in files {
                media_hint.push_str(&format!("- `{}`\n", f));
            }
            media_hint.push_str("\n");
        }
        *prompt = format!("{}{}", media_hint, prompt);
    }
}

pub(crate) async fn process_text_with_files(
    bot: &Bot,
    msg: &Message,
    text: &str,
    pre_downloaded_files: &[String],
    config: &CliConfig,
    sessions: &std::sync::Arc<SessionManager>,
    cli: &AntigravityCli,
    cron_manager: &crate::cron::manager::CronManager,
    topic_cache: &TopicNameCache,
) -> Result<(), teloxide::RequestError> {
    if commands::handle_commands(bot, msg, text, config, sessions.as_ref(), cli, cron_manager, topic_cache).await? { return Ok(()); }

    let (m_over, current_text) = parse_model_directive(text);
    let key = crate::session::key::SessionKey::telegram(msg.chat.id.0, get_topic_id(msg));
    let mut m = config.model.clone().unwrap_or_else(|| "antigravity-default".to_string());
    if let Some(ref mo) = m_over { m = mo.clone(); }

    let (mut sess, _) = sessions.resolve_session(&key, &config.provider, &m).await.unwrap();
    if let Some(ref mo) = m_over {
        if handle_model_override(bot, msg, mo, &mut sess, sessions.as_ref(), current_text.is_empty()).await? { return Ok(()); }
    }

    let active_session_id = session_init::initialize_session_if_needed(bot, msg, sessions, &mut sess, cli, config).await?;
    if active_session_id.is_empty() {
        return Ok(());
    }

    let mut prompt = build_reply_prompt(msg, current_text);
    inject_pre_downloaded_files(&mut prompt, pre_downloaded_files);

    history::log_telegram_message(&config.working_dir, &active_session_id, "user", Some(msg.id.0), text, true, None);
    if ask_helpers::feed_active_session_if_running(bot, msg, &active_session_id, current_text, cli, sessions, sess.clone(), config).await? { return Ok(()); }

    run_cli_stream(bot, msg, &prompt, &active_session_id, cli, sessions.as_ref(), sess, config).await
}

async fn process_text(
    bot: &Bot,
    msg: &Message,
    text: &str,
    config: &CliConfig,
    sessions: &std::sync::Arc<SessionManager>,
    cli: &AntigravityCli,
    cron_manager: &crate::cron::manager::CronManager,
    topic_cache: &TopicNameCache,
) -> Result<(), teloxide::RequestError> {
    let mut files = Vec::new();
    if reply::has_media(msg) {
        let dest_dir = config.working_dir.join("telegram_files");
        if let Ok(Some(f)) = reply::download_telegram_media(bot, msg, &dest_dir).await {
            files.push(f);
        }
    }
    process_text_with_files(bot, msg, text, &files, config, sessions, cli, cron_manager, topic_cache).await
}

pub mod handler;
pub(crate) use handler::handle_message;

pub use runner::run_bot;
