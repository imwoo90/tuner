//! # Telegram Bot Interface Module
//!
//! ## Overview
//! Handles Telegram updates, slash commands, PTY streaming previews, and interactive UI menus.
//!
//! ## Submodules
//! - [`formatting`] (`formatting/mod.rs`): Markdown-to-HTML conversion and chunk splitting.
//! - [`reply`] (`reply.rs`): Reply context extraction and model directive parsing.
//! - [`session_init`] (`session_init.rs`): Lazy session initialization and topic seed synthesis.
//! - [`history`] (`history.rs`): Persistent JSONL message logging.
//! - [`handover`] (`handover.rs`): Session handover summary extraction across `/new` resets.
//! - [`handover_helpers`] (`handover_helpers.rs`): Transcript parsing helpers for handover.
//! - [`commands`] (`commands.rs`): Primary slash command router.
//! - [`cron_selector`] (`cron_selector.rs`): Inline keyboard selector for cron tasks.
//! - [`cron_formatter`] (`cron_formatter.rs`): Human-readable cron schedule formatting.
//! - [`stream`] (`stream.rs`): Debounced PTY streaming and collapsible progress updates.
//! - [`transport`] (`transport.rs`): Adapts bus envelopes into Teloxide API calls.
//! - [`lang`] (`lang.rs`): Language selection menu and locale persistence.
//! - [`callbacks`] (`callbacks.rs`): Callback query dispatcher for inline buttons.
//! - [`ask_callbacks`] (`ask_callbacks.rs`): Callback handlers for `ask_question` prompts.
//! - [`ask_helpers`] (`ask_helpers.rs`): Active session prompt feeding helpers.
//! - [`ask_process`] (`ask_process.rs`): Multi-step `ask_question` state machine.
//! - [`multi_select`] (`multi_select.rs`): Multi-option toggle state for keyboards.
//! - [`runner`] (`runner.rs`): Teloxide dispatcher setup and polling loop.
//! - [`attachments`] (`attachments.rs`): Outbound file attachment extraction and delivery.
//! - [`attachments_dl`] (`attachments_dl.rs`): Direct attachment ZIP and file download handler.
//! - [`review`] (`review.rs`): Web file viewer session manager.
//! - [`review_server`] (`review_server.rs`): Embedded HTTP server for file and media review.
//! - [`review_store`] (`review_store.rs`): Persistent token store for reviewable files.
//! - [`review_callback`] (`review_callback.rs`): Callback handler for file review previews.
//! - [`media_group`] (`media_group.rs`): Multi-file album upload aggregator.
//! - [`upgrade`] (`upgrade.rs`): Self-upgrade command workflow.
//! - [`async_observer`] (`async_observer.rs`): Watcher for async CLI turn completions.
//! - [`commands_usage`] (`commands_usage.rs`): `/usage` quota report formatter.
//! - [`commands_session`] (`commands_session.rs`): `/new`, `/stop`, and `/abort` handlers.
//! - [`commands_remote_hub`] (`commands_remote_hub.rs`): Remote service detector and spawner.
//! - [`commands_remote`] (`commands_remote.rs`): `/remote` dashboard controller.
//! - [`typing`] (`typing.rs`): Scoped Telegram typing indicator guard.
//! - [`rate_limiter`] (`rate_limiter.rs`): Outbound per-chat rate limiter.
//! - [`callbacks_effort`] (`callbacks_effort.rs`): Model and reasoning effort callbacks.
//! - [`media`] (`media.rs`): Inbound media download and prompt injection.
//! - [`commands_model`] (`commands_model.rs`): `/model` and `/effort` command handlers.
//! - [`commands_registry`] (`commands_registry.rs`): Bot command registration and classification.
//! - [`auth`] (`auth.rs`): User/group authorization and topic migration checks.
//! - [`handler`] (`handler.rs`): Top-level update handler and concurrency gate.
//! - [`vault`] (`vault.rs`): Ephemeral Web Vault session manager and credential CRUD endpoints.
//!
//! ## Search Tags
//! #telegram-bot, #message-routing, #pty-streaming, #inline-keyboards, #chat-commands

use teloxide::prelude::*;
use crate::config::CliConfig;
use crate::cli::{antigravity::AntigravityCli, AgentProvider};
use crate::session::{manager::SessionManager, data::SessionData};

pub mod formatting;
pub mod vault;
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
pub mod formatting_tests;
pub mod reply;
pub mod session_init;
pub mod history;
pub mod handover;
pub mod handover_helpers;
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
pub mod handover_tests;
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
pub mod handover_integration_tests;
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
pub mod reply_tests;
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
pub mod reply_extra_tests;
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
/// Test module/item.
pub mod handler_tests;
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
/// Test module/item.
pub mod media_tests;
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
/// Test module/item.
pub mod forum_tests;
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
/// Test module/item.
pub mod ask_abort_tests;
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
pub mod session_init_tests;
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
pub mod test_helpers;
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
/// Test module/item.
pub static TEST_ENV_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());
pub mod commands;
pub mod cron_selector;
pub mod cron_formatter;
/// Forum topic metadata cache re-exports.
pub mod topic_cache {
    pub use super::auth::{BotInfo, TopicNameCache};
}
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
pub mod attachments_dl;
pub mod review;
pub mod review_server;
pub mod review_store;
pub mod review_callback;
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
/// Test module/item.
pub mod review_tests;
pub mod media_group;
pub mod upgrade;
pub mod async_observer;
pub mod commands_usage;
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
pub mod commands_usage_tests;
pub mod commands_session;
pub mod commands_remote_hub;
pub mod commands_remote;
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
/// Test module/item.
pub mod commands_remote_tests;

pub(crate) use reply::{build_reply_prompt, parse_model_directive};
pub use transport::TelegramTransport;
pub mod typing;
pub mod rate_limiter;


pub use reply::get_topic_id;

pub use topic_cache::{BotInfo, TopicNameCache};

pub(crate) async fn run_cli_stream(
    bot: &Bot,
    msg: &Message,
    prompt: &str,
    sid: &str,
    cli: &AntigravityCli,
    sess_ctx: (&SessionManager, SessionData),
    config: &CliConfig,
) -> Result<(), teloxide::RequestError> {
    let (sessions, sess) = sess_ctx;
    let tok = std::env::var("TELEGRAM_TOKEN").unwrap_or_else(|_| config.telegram_token.clone());
    let _g = typing::TelegramTypingGuard::new(bot.clone(), tok, msg).await;
    let mut cc = cli.clone();
    cc.config.chat_id = msg.chat.id.0;
    cc.config.topic_id = msg.thread_id.map(|t| t.0.0 as i64);
    if !sess.model.is_empty() { cc.config.model = Some(sess.model.clone()); }
    if let Some(ref effort) = sess.effort
        && !effort.is_empty() {
            cc.config.effort = Some(effort.clone());
        }
    match cc.send_streaming(prompt, (!sid.is_empty()).then_some(sid), false, config.working_dir.clone()).await {
        Ok(s) => stream::consume_stream(bot, (msg.chat.id, msg.thread_id.map(|t| t.0.0)), s, sessions, sess, &cc.config, cli).await?,
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
    model_and_effort: (&str, Option<&str>),
    sess: &mut crate::session::data::SessionData,
    sessions: &SessionManager,
    cli: &AntigravityCli,
    empty: bool,
) -> Result<bool, teloxide::RequestError> {
    let (mo, eff) = model_and_effort;
    sess.model = mo.to_string();
    sess.effort = eff.map(|s| s.to_string());
    let _ = sessions.preserve_session_identity(sess).await;
    let sid = sess.get_session_id(&sess.provider);
    commands_model::apply_model_to_active_pty(cli, &sid, &sess.model, sess.effort.as_deref()).await;
    if empty {
        let display = if let Some(e) = eff {
            format!("{} (effort: {})", mo, e)
        } else {
            mo.to_string()
        };
        let mut r = bot.send_message(msg.chat.id, format!("Next message will use {}", display));
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
            media_hint.push('\n');
        }
        *prompt = format!("{}{}", media_hint, prompt);
    }
}

async fn queue_pending_files(
    msg: &Message,
    files: &[String],
    config: &CliConfig,
    sessions: &SessionManager,
) {
    if files.is_empty() {
        return;
    }
    let key = crate::session::key::SessionKey::telegram(msg.chat.id.0, get_topic_id(msg));
    let default_model = config.model.as_deref().unwrap_or("antigravity-default");
    if let Ok((mut sess, _)) = sessions.resolve_session(&key, &config.provider, default_model).await {
        for f in files {
            if !sess.pending_attachments.contains(f) {
                sess.pending_attachments.push(f.clone());
            }
        }
        let _ = sessions.preserve_session_identity(&sess).await;
        let tok = std::env::var("TELEGRAM_TOKEN").unwrap_or_else(|_| config.telegram_token.clone());
        typing::set_emoji_reaction(msg.id.0, msg.chat.id, tok, "👍");
    }
}

pub(crate) fn strip_new_or_reset_prefix(text: &str) -> &str {
    let trimmed = text.trim();
    let Some(raw_cmd) = trimmed.split_whitespace().next() else {
        return text;
    };
    let cmd = raw_cmd.split('@').next().unwrap_or(raw_cmd);
    if cmd == "/new" || cmd == "/reset" {
        trimmed[raw_cmd.len()..].trim()
    } else {
        text
    }
}

async fn drain_session_attachments(
    bot: &Bot,
    msg: &Message,
    config: &CliConfig,
    sess: &mut crate::session::data::SessionData,
    pre_downloaded_files: &[String],
    sessions: &SessionManager,
) -> Vec<String> {
    let mut all_files = if !sess.pending_attachments.is_empty() {
        let drained = std::mem::take(&mut sess.pending_attachments);
        let _ = sessions.preserve_session_identity(sess).await;
        drained
    } else {
        Vec::new()
    };
    if let Some(replied) = msg.reply_to_message()
        && reply::has_media(replied)
    {
        let dest_dir = config.working_dir.join("telegram_files");
        if let Ok(Some(replied_file)) = reply::download_telegram_media(bot, replied, &dest_dir).await
            && !all_files.contains(&replied_file)
        {
            all_files.push(replied_file);
        }
    }
    for f in pre_downloaded_files {
        if !all_files.contains(f) {
            all_files.push(f.clone());
        }
    }
    all_files
}

async fn resolve_message_session<'a>(
    bot: &Bot,
    msg: &Message,
    text: &'a str,
    config: &CliConfig,
    sessions: &SessionManager,
    cli: &AntigravityCli,
) -> Result<Option<(crate::session::data::SessionData, &'a str)>, teloxide::RequestError> {
    let (m_over, eff_over, current_text) = parse_model_directive(text);
    let key = crate::session::key::SessionKey::telegram(msg.chat.id.0, get_topic_id(msg));
    let mut m = config.model.clone().unwrap_or_else(|| "antigravity-default".to_string());
    if let Some(ref mo) = m_over { m = mo.clone(); }

    let Ok((mut sess, _)) = sessions.resolve_session(&key, &config.provider, &m).await else {
        return Ok(None);
    };
    if let Some(ref mo) = m_over
        && handle_model_override(bot, msg, (mo, eff_over.as_deref()), &mut sess, sessions, cli, current_text.is_empty()).await? {
            return Ok(None);
        }
    Ok(Some((sess, current_text)))
}

pub(crate) async fn process_text_with_files(
    bot: &Bot,
    msg: &Message,
    payload: (&str, &[String]),
    config: &CliConfig,
    sessions: &std::sync::Arc<SessionManager>,
    cli: &AntigravityCli,
    aux: (&crate::cron::manager::CronManager, &TopicNameCache),
) -> Result<(), teloxide::RequestError> {
    let (raw_text, files) = payload;
    if raw_text.trim().is_empty() || commands::handle_commands(bot, msg, raw_text, config, sessions.as_ref(), cli, aux).await? {
        queue_pending_files(msg, files, config, sessions.as_ref()).await;
        return Ok(());
    }
    let text = strip_new_or_reset_prefix(raw_text);
    let Some((mut sess, current_text)) = resolve_message_session(bot, msg, text, config, sessions.as_ref(), cli).await? else {
        queue_pending_files(msg, files, config, sessions.as_ref()).await;
        return Ok(());
    };
    let escaped = commands_registry::escape_non_workflow_slash(current_text);
    let sid = session_init::initialize_session_if_needed(bot, msg, sessions, &mut sess, cli, config).await?;
    if sid.is_empty() {
        queue_pending_files(msg, files, config, sessions.as_ref()).await;
        return Ok(());
    }
    async_observer::spawn_session_async_observer(bot.clone(), msg, sid.clone(), cli.clone(), sessions.clone(), config.clone());
    history::log_telegram_message(&config.working_dir, &sid, (sess.topic_id, sess.topic_name.as_deref()), "user", Some(msg.id.0), text, (true, None));
    if ask_helpers::feed_active_session_if_running(bot, msg, &sid, &escaped, cli, (sessions, sess.clone()), config).await? {
        queue_pending_files(msg, files, config, sessions.as_ref()).await;
        return Ok(());
    }
    let all_files = drain_session_attachments(bot, msg, config, &mut sess, files, sessions.as_ref()).await;
    let mut prompt = build_reply_prompt(msg, &escaped);
    inject_pre_downloaded_files(&mut prompt, &all_files);
    run_cli_stream(bot, msg, &prompt, &sid, cli, (sessions.as_ref(), sess), config).await
}

pub mod callbacks_effort;
pub mod media;
pub mod commands_model;
pub mod commands_registry;
pub mod auth;
pub mod handler;
pub(crate) use handler::handle_message;
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
/// Test module/item.
pub mod lock_bypass_tests;

pub use runner::run_bot;
