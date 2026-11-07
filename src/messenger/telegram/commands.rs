//! # Telegram Slash Commands Controller
//!
//! ## Overview
//! Parses and handles Telegram slash commands (e.g. `/model`, `/new`, `/abort`, `/diagnose`, `/cron`).
//! Validates permissions and updates active workspace states.
//!
//! ## Search Tags
//! #chat-commands, #access-control, #session-management, #command-parser

use teloxide::prelude::*;
use crate::config::CliConfig;
use crate::cli::antigravity::AntigravityCli;
use crate::t;
use super::commands_model::{handle_model_command, handle_effort_command};
use super::commands_session::{handle_session_control_commands, handle_memory_command};
use super::callbacks_effort::handle_provider_command;
#[allow(unused_imports)]
pub(crate) use super::commands_registry::{get_bot_commands, register_commands, is_lock_free_command, handle_help_command};

pub(crate) async fn send_reply(
    bot: &Bot,
    msg: &Message,
    text: impl Into<String>,
) -> Result<Message, teloxide::RequestError> {
    let limiter = super::rate_limiter::global_chat_rate_limiter();
    let raw_str = text.into();
    let html_str = super::formatting::markdown_to_telegram_html(&raw_str);
    let res = limiter.execute_rate_limited(msg.chat.id, || {
        let mut req = bot.send_message(msg.chat.id, html_str.clone())
            .parse_mode(teloxide::types::ParseMode::Html);
        if let Some(tid) = msg.thread_id {
            req = req.message_thread_id(tid);
        }
        async move { req.await }
    }).await;
    if res.is_err() {
        let mut fallback = bot.send_message(msg.chat.id, raw_str);
        if let Some(tid) = msg.thread_id {
            fallback = fallback.message_thread_id(tid);
        }
        return fallback.await;
    }
    res
}

async fn append_topic_status(
    report: &mut String,
    msg: &Message,
    config: &CliConfig,
    sessions: &crate::session::manager::SessionManager,
    cli: &AntigravityCli,
) {
    let topic_id = crate::telegram::get_topic_id(msg);
    let key = crate::session::key::SessionKey::telegram(msg.chat.id.0, topic_id);
    if let Ok(Some(s)) = sessions.get_active(&key).await {
        if let Some(tid) = topic_id {
            let t_label = match s.topic_name.as_deref() {
                Some(name) if !name.is_empty() => format!("#{} ({})", tid, name),
                _ => format!("#{}", tid),
            };
            report.push_str(&format!("\n- Topic: {}", t_label));
        }
        let active_prov = if !s.provider.is_empty() { &s.provider } else { &config.provider };
        let sid = s.get_session_id(active_prov);
        if !sid.is_empty() {
            let short_sid = &sid[..sid.len().min(8)];
            let turns = s.provider_sessions.get(active_prov).map(|p| p.message_count).unwrap_or(0);
            let is_turn_running = cli.sessions.is_running(&sid).await
                || crate::cli::claude::is_running(msg.chat.id.0, topic_id, Some(&sid)).await
                || crate::cli::codex::is_running(msg.chat.id.0, topic_id, Some(&sid)).await;
            let state_str = if is_turn_running {
                "⚡ running"
            } else if cli.sessions.is_async_running(&sid).await {
                "⏳ async task"
            } else {
                "idle"
            };
            report.push_str(&format!("\n- Topic Session: `{}` ({} turns, {})", short_sid, turns, state_str));
        }
        if !s.pending_attachments.is_empty() {
            report.push_str(&format!("\n📎 Pending Attachments: {}", s.pending_attachments.len()));
        }
    } else if let Some(tid) = topic_id {
        report.push_str(&format!("\n- Topic: #{}", tid));
    }
}

async fn resolve_topic_provider(
    msg: &Message,
    config: &CliConfig,
    sessions: &crate::session::manager::SessionManager,
) -> String {
    let key = crate::session::key::SessionKey::telegram(msg.chat.id.0, crate::telegram::get_topic_id(msg));
    sessions.get_active(&key).await.ok().flatten()
        .map(|s| s.provider).filter(|p| !p.is_empty()).unwrap_or_else(|| config.provider.clone())
}

async fn handle_info_commands(
    bot: &Bot,
    msg: &Message,
    text: &str,
    config: &CliConfig,
    sessions: &crate::session::manager::SessionManager,
    cli: &AntigravityCli,
) -> Result<bool, teloxide::RequestError> {
    let (cmd, args) = super::commands_registry::split_slash_command(text);
    if !args.is_empty() {
        return Ok(false);
    }
    if cmd == "/help" || cmd == "/start" {
        let prov = resolve_topic_provider(msg, config, sessions).await;
        let _ = super::commands_registry::handle_help_command_for_provider(bot, msg, &prov).await;
        return Ok(true);
    }
    if cmd == "/status" || cmd == "/diagnose" {
        let agy_status = crate::cli::antigravity::events::agy_version_summary()
            .map(|v| t!("bot.diagnose_installed", version = v))
            .unwrap_or_else(|| t!("bot.diagnose_not_found"));
        let session_count = cli.sessions.active_count().await
            + crate::cli::claude::active_count().await + crate::cli::codex::active_count().await;
        let has_tok = std::env::var("TELEGRAM_TOKEN").is_ok() || !config.telegram_token.is_empty();
        let token_present = if has_tok { t!("bot.diagnose_token_set") } else { t!("bot.diagnose_token_missing") };
        let active_provider = resolve_topic_provider(msg, config, sessions).await;
        let model = crate::telegram::reply::resolve_session_model(msg, config, sessions).await;
        let mut report = t!("bot.status", agy_status = agy_status, token_present = token_present, session_count = session_count, provider = active_provider, model = model);
        append_topic_status(&mut report, msg, config, sessions, cli).await;
        let _ = send_reply(bot, msg, report).await;
        return Ok(true);
    }
    if cmd == "/restart" {
        let _ = send_reply(bot, msg, t!("bot.restart")).await;
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
        std::process::exit(42);
    }
    Ok(false)
}

async fn handle_pref_commands(
    bot: &Bot,
    msg: &Message,
    text: &str,
    config: &CliConfig,
    sessions: &crate::session::manager::SessionManager,
    cli: &AntigravityCli,
) -> bool {
    let (cmd, args) = super::commands_registry::split_slash_command(text);
    match cmd {
        "/provider" => {
            let _ = handle_provider_command(bot, msg, args, config, sessions).await;
            true
        }
        "/model" => {
            let _ = handle_model_command(bot, msg, args, config, sessions, cli).await;
            true
        }
        "/effort" => {
            let _ = handle_effort_command(bot, msg, args, config, sessions, cli).await;
            true
        }
        "/lang" => {
            let _ = crate::telegram::lang::handle_lang_command(bot, msg, args, config, sessions).await;
            true
        }
        _ => false,
    }
}

pub(crate) async fn handle_commands(
    bot: &Bot,
    msg: &Message,
    text: &str,
    config: &CliConfig,
    sessions: &std::sync::Arc<crate::session::manager::SessionManager>,
    cli: &AntigravityCli,
    aux: (&crate::cron::manager::CronManager, &super::TopicNameCache),
) -> Result<bool, teloxide::RequestError> {
    let (cron_manager, topic_cache) = aux;
    if handle_info_commands(bot, msg, text, config, sessions, cli).await? {
        return Ok(true);
    }
    let trimmed = text.trim();
    let (cmd, args) = super::commands_registry::split_slash_command(trimmed);
    if cmd == "/usage" && args.is_empty() {
        super::commands_usage::handle_usage_command(bot, msg, config, sessions, cli).await?;
        return Ok(true);
    }
    if cmd == "/remote" && args.is_empty() {
        super::commands_remote::handle_remote_command(bot, msg, config).await?;
        return Ok(true);
    }
    if cmd == "/vault" && args.is_empty() {
        super::vault::handle_vault_command(bot, msg, config).await?;
        return Ok(true);
    }
    if matches!(cmd, "/new" | "/reset") || (matches!(cmd, "/stop" | "/stop_all" | "/abort") && args.is_empty()) {
        return handle_session_control_commands(bot, msg, trimmed, config, sessions, cli, topic_cache).await;
    }
    if handle_pref_commands(bot, msg, trimmed, config, sessions, cli).await {
        return Ok(true);
    }
    if cmd == "/memory" && args.is_empty() {
        let _ = handle_memory_command(bot, msg, config, sessions, cli).await;
        return Ok(true);
    }
    if cmd == "/cron" && args.is_empty() {
        let _ = handle_cron_command(bot, msg, cron_manager, topic_cache).await;
        return Ok(true);
    }
    if cmd == "/upgrade" && args.is_empty() {
        let _ = super::upgrade::handle_upgrade_command(bot, msg).await;
        return Ok(true);
    }
    Ok(false)
}

async fn handle_cron_command(
    bot: &Bot,
    msg: &Message,
    cron_manager: &crate::cron::manager::CronManager,
    topic_cache: &super::TopicNameCache,
) -> Result<(), teloxide::RequestError> {
    if let Ok((txt, markup)) = crate::telegram::cron_selector::build_cron_page(cron_manager, 0, None, Some(topic_cache)).await {
        let mut req = bot.send_message(msg.chat.id, txt).parse_mode(teloxide::types::ParseMode::Html);
        if let Some(tid) = msg.thread_id { req = req.message_thread_id(tid); }
        let _ = req.reply_markup(markup).await;
    }
    Ok(())
}

