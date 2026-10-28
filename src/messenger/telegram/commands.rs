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
use super::commands_session::handle_session_control_commands;
#[allow(unused_imports)]
pub(crate) use super::commands_registry::{get_bot_commands, register_commands, is_lock_free_command, handle_help_command};

pub(crate) async fn send_reply(
    bot: &Bot,
    msg: &Message,
    text: impl Into<String>,
) -> Result<Message, teloxide::RequestError> {
    let limiter = super::rate_limiter::global_chat_rate_limiter();
    let text_str = text.into();
    let res = limiter.execute_rate_limited(msg.chat.id, || {
        let mut req = bot.send_message(msg.chat.id, text_str.clone());
        if let Some(tid) = msg.thread_id {
            req = req.message_thread_id(tid);
        }
        async move { req.await }
    }).await;
    if let Err(ref e) = res {
        eprintln!(
            "❌ [tuner] Failed to send reply (chat: {}, thread: {:?}): {:?}",
            msg.chat.id,
            msg.thread_id.map(|t| t.0.0),
            e
        );
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
        let sid = s.get_session_id(&config.provider);
        if !sid.is_empty() {
            let short_sid = &sid[..sid.len().min(8)];
            let turns = s.provider_sessions.get(&config.provider).map(|p| p.message_count).unwrap_or(0);
            let state_str = if cli.sessions.is_running(&sid).await {
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

async fn handle_info_commands(
    bot: &Bot,
    msg: &Message,
    text: &str,
    config: &CliConfig,
    sessions: &crate::session::manager::SessionManager,
    cli: &AntigravityCli,
) -> Result<bool, teloxide::RequestError> {
    let trimmed = text.trim();
    if trimmed == "/help" || trimmed == "/start" {
        let _ = handle_help_command(bot, msg).await;
        return Ok(true);
    }
    if trimmed == "/status" || trimmed == "/diagnose" {
        let agy_status = std::process::Command::new("agy").arg("--version").output()
            .map(|o| t!("bot.diagnose_installed", version = String::from_utf8_lossy(&o.stdout).trim()))
            .unwrap_or_else(|_| t!("bot.diagnose_not_found"));
        let session_count = cli.sessions.active_count().await;
        let token_present = if std::env::var("TELEGRAM_TOKEN").is_ok() || !config.telegram_token.is_empty() {
            t!("bot.diagnose_token_set")
        } else {
            t!("bot.diagnose_token_missing")
        };
        let mut report = t!(
            "bot.status",
            agy_status = agy_status,
            token_present = token_present,
            session_count = session_count,
            provider = config.provider,
            model = crate::telegram::reply::resolve_session_model(msg, config, sessions).await
        );
        append_topic_status(&mut report, msg, config, sessions, cli).await;
        let _ = send_reply(bot, msg, report).await;
        return Ok(true);
    }
    if trimmed == "/restart" {
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
    if let Some(args) = text.strip_prefix("/model") {
        let _ = handle_model_command(bot, msg, args.trim(), config, sessions, cli).await;
        return true;
    }
    if let Some(args) = text.strip_prefix("/effort") {
        let _ = handle_effort_command(bot, msg, args.trim(), config, sessions, cli).await;
        return true;
    }
    if let Some(args) = text.strip_prefix("/lang") {
        let _ = crate::telegram::lang::handle_lang_command(bot, msg, args.trim(), config, sessions).await;
        return true;
    }
    false
}

pub(crate) async fn handle_commands(
    bot: &Bot,
    msg: &Message,
    text: &str,
    config: &CliConfig,
    sessions: &crate::session::manager::SessionManager,
    cli: &AntigravityCli,
    aux: (&crate::cron::manager::CronManager, &super::TopicNameCache),
) -> Result<bool, teloxide::RequestError> {
    let (cron_manager, topic_cache) = aux;
    if handle_info_commands(bot, msg, text, config, sessions, cli).await? {
        return Ok(true);
    }
    let trimmed = text.trim();
    if trimmed == "/usage" {
        super::commands_usage::handle_usage_command(bot, msg, config, sessions, cli).await?;
        return Ok(true);
    }
    if trimmed == "/remote" || trimmed.starts_with("/remote ") {
        super::commands_remote::handle_remote_command(bot, msg, config).await?;
        return Ok(true);
    }
    if trimmed == "/vault" || trimmed.starts_with("/vault ") || trimmed.starts_with("/vault@") {
        super::vault::handle_vault_command(bot, msg, config).await?;
        return Ok(true);
    }
    if trimmed.starts_with("/new") || trimmed.starts_with("/reset") || trimmed == "/stop" || trimmed == "/stop_all" || trimmed == "/abort" {
        return handle_session_control_commands(bot, msg, trimmed, config, sessions, cli, topic_cache).await;
    }
    if handle_pref_commands(bot, msg, trimmed, config, sessions, cli).await {
        return Ok(true);
    }
    if trimmed == "/memory" || trimmed.starts_with("/memory@") {
        let _ = handle_memory_command(bot, msg, config, sessions, cli).await;
        return Ok(true);
    }
    if trimmed == "/cron" {
        let _ = handle_cron_command(bot, msg, cron_manager, topic_cache).await;
        return Ok(true);
    }
    if trimmed == "/upgrade" {
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

async fn try_query_live_memory(
    bot: &Bot,
    msg: &Message,
    config: &CliConfig,
    sessions: &crate::session::manager::SessionManager,
    cli: &AntigravityCli,
) -> bool {
    let key = crate::session::key::SessionKey::telegram(msg.chat.id.0, crate::telegram::get_topic_id(msg));
    let model = config.model.as_deref().unwrap_or("antigravity-default");
    if let Ok((mut sess, _)) = sessions.resolve_session(&key, &config.provider, model).await
        && let Ok(sid) = super::session_init::initialize_session_if_needed(bot, msg, sessions, &mut sess, cli, config).await
        && !sid.is_empty() && !sid.starts_with("mock-")
    {
        let prompt = "현재 세션의 시스템 프롬프트 `<user_rules>`(Rules Context)에 주입되어 있는 `mainmemory.md` 내용(About the User, Core System Architecture & Roles, Decisions & Preferences)과 적용된 룰 상태를 요약 없이 있는 그대로 출력해줘.";
        let _ = super::run_cli_stream(bot, msg, prompt, &sid, cli, (sessions, sess), config).await;
        return true;
    }
    false
}

async fn send_static_memory_fallback(bot: &Bot, msg: &Message, config: &CliConfig) -> Result<(), teloxide::RequestError> {
    let p1 = config.working_dir.join(".agents/rules/mainmemory.md");
    let path = if p1.is_file() { p1 } else { config.working_dir.join("memory_system/MAINMEMORY.md") };
    let raw = std::fs::read_to_string(path).unwrap_or_else(|_| t!("bot.memory_empty"));
    let content = raw.strip_prefix("---").and_then(|s| s.find("---").map(|i| s[i + 3..].trim_start())).unwrap_or(&raw);
    let html = crate::telegram::formatting::markdown_to_telegram_html(content);
    for chunk in crate::telegram::formatting::split_html_message(&html, 4000) {
        let mut req = bot.send_message(msg.chat.id, chunk).parse_mode(teloxide::types::ParseMode::Html);
        if let Some(tid) = msg.thread_id { req = req.message_thread_id(tid); }
        let _ = req.await;
    }
    Ok(())
}

async fn handle_memory_command(
    bot: &Bot,
    msg: &Message,
    config: &CliConfig,
    sessions: &crate::session::manager::SessionManager,
    cli: &AntigravityCli,
) -> Result<(), teloxide::RequestError> {
    if try_query_live_memory(bot, msg, config, sessions, cli).await {
        return Ok(());
    }
    send_static_memory_fallback(bot, msg, config).await
}

