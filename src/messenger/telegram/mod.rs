//! # Telegram Bot Interface Module
//!
//! ## Overview
//! Handles Telegram updates, slash commands, PTY streaming previews, and interactive UI menus.
//!
//! ## Submodules
//! - [`formatting`] (`formatting/mod.rs`): Markdown-to-HTML and chunk splitting.
//! - [`reply`] (`reply.rs`): Reply context extraction and mention filtering.
//! - [`reply_directives`] (`reply_directives.rs`): `@model` directives and session model display.
//! - [`session_init`] (`session_init.rs`): Lazy session boot and topic seed synthesis.
//! - [`history`] (`history.rs`): Persistent JSONL message logging.
//! - [`handover`] (`handover.rs`): Handover summary across `/new` resets.
//! - [`handover_helpers`] (`handover_helpers.rs`): Transcript helpers for handover.
//! - [`commands`] (`commands.rs`): Primary slash command router.
//! - [`cron_selector`] (`cron_selector.rs`): Inline keyboard selector for cron tasks.
//! - [`cron_formatter`] (`cron_formatter.rs`): Human-readable cron formatting.
//! - [`stream`] (`stream.rs`): Debounced PTY streaming and progress updates.
//! - [`transport`] (`transport.rs`): Adapts bus envelopes into Teloxide API calls.
//! - [`lang`] (`lang.rs`): Language menu and locale persistence.
//! - [`callbacks`] (`callbacks.rs`): Callback query dispatcher.
//! - [`ask_callbacks`] (`ask_callbacks.rs`): Callback handlers for `ask_question`.
//! - [`ask_helpers`] (`ask_helpers.rs`): Active session prompt feeding helpers.
//! - [`ask_process`] (`ask_process.rs`): Multi-step `ask_question` state machine.
//! - [`multi_select`] (`multi_select.rs`): Multi-option keyboard toggle state.
//! - [`runner`] (`runner.rs`): Teloxide dispatcher setup and polling loop.
//! - [`attachments`] (`attachments.rs`): Outbound file delivery.
//! - [`attachments_dl`] (`attachments_dl.rs`): Attachment ZIP/file downloader.
//! - [`review`] (`review.rs`): Web viewer session manager.
//! - [`review_server`] (`review_server.rs`): HTTP server for file review.
//! - [`review_store`] (`review_store.rs`): Reviewable file token store.
//! - [`review_callback`] (`review_callback.rs`): Review preview callbacks.
//! - [`media_group`] (`media_group.rs`): Album upload aggregator.
//! - [`upgrade`] (`upgrade.rs`): Self-upgrade command workflow.
//! - [`async_observer`] (`async_observer.rs`): Async turn completion watcher.
//! - [`commands_usage`] (`commands_usage.rs`): `/usage` dispatcher.
//! - [`commands_usage_agy`] (`commands_usage_agy.rs`): CloudCode quota for `agy`.
//! - [`commands_usage_claude`] (`commands_usage_claude.rs`): Quota/JSONL usage for `claude`.
//! - [`commands_usage_codex`] (`commands_usage_codex.rs`): RPC/JSONL usage for `codex`.
//! - [`commands_session`] (`commands_session.rs`): `/new`, `/stop`, `/abort` handlers.
//! - [`commands_remote_hub`] (`commands_remote_hub.rs`): Remote service spawner.
//! - [`commands_remote`] (`commands_remote.rs`): `/remote` dashboard controller.
//! - [`typing`] (`typing.rs`): Typing indicator guard.
//! - [`rate_limiter`] (`rate_limiter.rs`): Per-chat rate limiter.
//! - [`callbacks_effort`] (`callbacks_effort.rs`): Model/effort callbacks.
//! - [`commands_provider`] (`commands_provider.rs`): `/provider` handler.
//! - [`media`] (`media.rs`): Media download and prompt injection.
//! - [`commands_model`] (`commands_model.rs`): `/model` and `/effort` handlers.
//! - [`commands_registry`] (`commands_registry.rs`): Bot command registry.
//! - [`auth`] (`auth.rs`): Auth and topic migration.
//! - [`message_session`] (`message_session.rs`): Turn-gated session & directive resolver.
//! - [`handler`] (`handler.rs`): Update handler and concurrency gate.
//! - [`vault`] (`vault.rs`): Web Vault CRUD endpoints.
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
pub mod reply_directives;
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
pub mod commands_usage_agy;
pub mod commands_usage_claude;
pub mod commands_usage_codex;
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

pub(crate) use reply::build_reply_prompt;
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
    let _fg = crate::upgrade::begin_foreground_run();
    let _turn_g = cli.begin_topic_turn(msg.chat.id.0, sess.topic_id.or_else(|| get_topic_id(msg)));
    let tok = std::env::var("TELEGRAM_TOKEN").unwrap_or_else(|_| config.telegram_token.clone());
    let _g = typing::TelegramTypingGuard::new(bot.clone(), tok, msg).await;
    let tid = sess.topic_id.or_else(|| get_topic_id(msg));
    let cc = session_init::configure_session_cli(cli, &sess, msg.chat.id.0, tid).await;
    match cc.send_streaming(prompt, (!sid.is_empty()).then_some(sid), false, config.working_dir.clone()).await {
        Ok(s) => stream::consume_stream(bot, (msg.chat.id, msg.thread_id.map(|t| t.0.0)), s, sessions, sess, &cc.config, cli).await?,
        Err(e) => {
            eprintln!("CLI ERROR: {:?}", e);
            let err_msg = crate::cli::antigravity::error_parser::parse_cli_error(&e, 1, cc.config.model.as_deref());
            let _ = commands::send_reply(bot, msg, err_msg).await;
        }
    }
    Ok(())
}

use media::{build_prompt_with_attachments, queue_pending_files};

async fn try_feed_open_ask_message(
    bot: &Bot,
    msg: &Message,
    payload: (&str, &[String]),
    config: &CliConfig,
    sessions: &std::sync::Arc<SessionManager>,
    cli: &AntigravityCli,
) -> Result<bool, teloxide::RequestError> {
    let (text, files) = payload;
    if auth::has_leading_directive(msg, text, config, sessions).await {
        return Ok(false);
    }
    let Some(active_sess) = auth::get_live_open_ask_session(msg, config, sessions, cli).await else {
        return Ok(false);
    };
    let escaped = commands_registry::escape_non_workflow_slash(text.trim());
    let p = SessionData::effective_provider(&active_sess.provider, &config.provider);
    let sid = active_sess.get_session_id(p);
    history::log_telegram_message(&config.working_dir, &sid, (active_sess.topic_id, active_sess.topic_name.as_deref()), "user", Some(msg.id.0), text, (true, None));
    crate::messenger::web::events_bridge::tg_user(&active_sess, text).await;
    if ask_helpers::feed_active_session_if_running(bot, msg, &sid, &escaped, cli, (sessions, active_sess), config).await? {
        queue_pending_files(msg, files, config, sessions).await;
        return Ok(true);
    }
    Ok(false)
}

async fn execute_resolved_turn(
    bot: &Bot,
    msg: &Message,
    payload: (&str, &str, &[String]),
    mut sess: SessionData,
    config: &CliConfig,
    sessions: &std::sync::Arc<SessionManager>,
    cli: &AntigravityCli,
) -> Result<(), teloxide::RequestError> {
    let (raw_text, current_text, files) = payload;
    let escaped = commands_registry::escape_non_workflow_slash(current_text);
    let mut sid = session_init::ensure_session_ready_for_turn(bot, msg, sessions, &mut sess, cli, config).await?;
    if sid.is_empty() {
        queue_pending_files(msg, files, config, sessions).await;
        return Ok(());
    }
    history::log_telegram_message(&config.working_dir, &sid, (sess.topic_id, sess.topic_name.as_deref()), "user", Some(msg.id.0), raw_text, (true, None));
    crate::messenger::web::events_bridge::tg_user(&sess, raw_text).await;
    if ask_helpers::feed_active_session_if_running(bot, msg, &sid, &escaped, cli, (sessions, sess.clone()), config).await? {
        queue_pending_files(msg, files, config, sessions).await;
        return Ok(());
    }
    if let Ok(Some(latest)) = sessions.get_active(&sess.session_key()).await {
        sess = latest;
    }
    let cur_prov = SessionData::effective_provider(&sess.provider, &config.provider);
    if sess.get_session_id(cur_prov) != sid {
        sid = session_init::ensure_session_ready_for_turn(bot, msg, sessions, &mut sess, cli, config).await?;
        if sid.is_empty() {
            queue_pending_files(msg, files, config, sessions).await;
            return Ok(());
        }
    }
    let prompt = build_prompt_with_attachments(bot, msg, &escaped, files, config, &mut sess, sessions).await;
    run_cli_stream(bot, msg, &prompt, &sid, cli, (sessions, sess), config).await
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
    let (text, files) = payload;
    if text.trim().is_empty() || commands::handle_commands(bot, msg, text, config, sessions, cli, aux).await? {
        queue_pending_files(msg, files, config, sessions).await;
        return Ok(());
    }
    if try_feed_open_ask_message(bot, msg, payload, config, sessions, cli).await? {
        return Ok(());
    }
    let Some((sess, current_text)) = auth::resolve_message_session(bot, msg, text, config, sessions, cli).await? else {
        queue_pending_files(msg, files, config, sessions).await;
        return Ok(());
    };
    if try_feed_open_ask_message(bot, msg, (current_text, files), config, sessions, cli).await? {
        return Ok(());
    }
    execute_resolved_turn(bot, msg, (text, current_text, files), sess, config, sessions, cli).await
}

pub mod callbacks_effort;
pub mod commands_provider;
pub mod media;
pub mod commands_model;
pub mod commands_registry;
pub mod auth;
pub mod message_session;
pub mod handler;
pub(crate) use handler::handle_message;
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
/// Test module/item.
pub mod lock_bypass_tests;
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
pub mod round9_audit_tests;
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
pub mod round11_audit_tests;
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
pub mod round14_audit_tests;

pub use runner::run_bot;
