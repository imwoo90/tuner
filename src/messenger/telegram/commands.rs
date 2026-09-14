//! # Telegram Slash Commands Controller
//!
//! ## Overview
//! Parses and handles Telegram slash commands (e.g. `/model`, `/new`, `/abort`, `/diagnose`, `/cron`).
//! Validates permissions and updates active workspace states.
//!
//! ## Collaboration Graph
//! - Called by [`process_text_with_files`](super::process_text_with_files) at early check cycles.
//! - Instructs [`SessionManager`](crate::session::manager::SessionManager) to reset or resolve sessions.
//! - Interacts with [`CronManager`](crate::cron::manager::CronManager) to toggle scheduled jobs.
//!
//! ## Search Tags
//! #chat-commands, #access-control, #session-management, #command-parser

use teloxide::prelude::*;
use crate::config::CliConfig;
use crate::cli::antigravity::AntigravityCli;
use crate::t;
use super::commands_model::{handle_model_command, handle_effort_command};
#[allow(unused_imports)]
pub(crate) use super::commands_registry::{get_bot_commands, register_commands, is_lock_free_command};

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

async fn handle_help_command(
    bot: &Bot,
    msg: &Message,
) -> Result<(), teloxide::RequestError> {
    let help_text = "\
🤖 <b>[tuner] Command Reference:</b>

• /status - System diagnostics & session info
• /stop - Cancel active task in this topic (ESC)
• /abort - Hard kill all running worker processes
• /new (/reset) - Start a fresh conversation
• /model - Switch active AI model
• /effort - Set reasoning effort (low, medium, high)
• /lang - Change interface language
• /memory - View persistent MAINMEMORY.md
• /cron - Manage scheduled cron tasks
• /usage - View model quota and remaining limits
• /upgrade - Check for updates and self-upgrade
• /restart - Request clean service restart";
    let _ = send_reply(bot, msg, help_text).await;
    Ok(())
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
        let agy_status = match std::process::Command::new("agy").arg("--version").output() {
            Ok(out) => {
                let ver = String::from_utf8_lossy(&out.stdout).trim().to_string();
                t!("bot.diagnose_installed", version = ver)
            }
            Err(_) => t!("bot.diagnose_not_found"),
        };
        let session_count = cli.sessions.active_count().await;
        let token_present = if std::env::var("TELEGRAM_TOKEN").is_ok() || !config.telegram_token.is_empty() {
            t!("bot.diagnose_token_set")
        } else {
            t!("bot.diagnose_token_missing")
        };
        let model_str = crate::telegram::reply::resolve_session_model(msg, config, sessions).await;
        let report = t!(
            "bot.status",
            agy_status = agy_status,
            token_present = token_present,
            session_count = session_count,
            provider = config.provider,
            model = model_str
        );
        let _ = send_reply(bot, msg, report).await;
        return Ok(true);
    }
    let trimmed = text.trim();
    if trimmed == "/restart" {
        let _ = send_reply(bot, msg, t!("bot.restart")).await;
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
        std::process::exit(42);
    }
    Ok(false)
}

pub(crate) async fn handle_commands(
    bot: &Bot,
    msg: &Message,
    text: &str,
    config: &CliConfig,
    sessions: &crate::session::manager::SessionManager,
    cli: &AntigravityCli,
    cron_manager: &crate::cron::manager::CronManager,
    topic_cache: &super::TopicNameCache,
) -> Result<bool, teloxide::RequestError> {
    if handle_info_commands(bot, msg, text, config, sessions, cli).await? {
        return Ok(true);
    }
    let trimmed = text.trim();
    if trimmed == "/usage" {
        super::commands_usage::handle_usage_command(bot, msg, config, sessions, cli, topic_cache).await?;
        return Ok(true);
    }
    if text.starts_with("/new") || text.starts_with("/reset") || trimmed == "/stop" || trimmed == "/stop_all" || trimmed == "/abort" {
        return handle_session_control_commands(bot, msg, text, config, sessions, cli, topic_cache).await;
    }
    if text.starts_with("/model") {
        let args = text["/model".len()..].trim();
        let _ = handle_model_command(bot, msg, args, config, sessions, cli).await;
        return Ok(true);
    }
    if text.starts_with("/effort") {
        let args = text["/effort".len()..].trim();
        let _ = handle_effort_command(bot, msg, args, config, sessions).await;
        return Ok(true);
    }
    if text.starts_with("/lang") {
        let args = text["/lang".len()..].trim();
        let _ = crate::telegram::lang::handle_lang_command(bot, msg, args, config, sessions).await;
        return Ok(true);
    }
    if trimmed == "/memory" {
        let _ = handle_memory_command(bot, msg, config).await;
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
    match crate::telegram::cron_selector::build_cron_page(cron_manager, 0, None, Some(topic_cache)).await {
        Ok((txt, markup)) => {
            let mut req = bot.send_message(msg.chat.id, txt)
                .parse_mode(teloxide::types::ParseMode::Html);
            if let Some(tid) = msg.thread_id {
                req = req.message_thread_id(tid);
            }
            if let Err(e) = req.reply_markup(markup).await {
                eprintln!("❌ [tuner] Failed to send cron reply markup: {:?}", e);
            }
        }
        Err(e) => {
            eprintln!("❌ [tuner] handle_cron_command error: {}", e);
            let _ = send_reply(bot, msg, &format!("❌ Failed to load cron jobs: {}", e)).await;
        }
    }
    Ok(())
}



async fn handle_new_command(
    bot: &Bot,
    msg: &Message,
    args: &str,
    mut topic_id: Option<i64>,
    config: &CliConfig,
    sessions: &crate::session::manager::SessionManager,
    cli: &AntigravityCli,
    topic_cache: &super::TopicNameCache,
) -> Result<(), teloxide::RequestError> {
    let parts: Vec<&str> = args.split_whitespace().collect();
    if parts.len() > 1 {
        let name = parts[1];
        if let Some(resolved_tid) = topic_cache.find_by_name(msg.chat.id.0, name) {
            topic_id = Some(resolved_tid);
        } else {
            let _ = send_reply(bot, msg, t!("bot.unknown_topic", name = name)).await;
            return Ok(());
        }
    }
    let key = crate::session::key::SessionKey::telegram(msg.chat.id.0, topic_id);
    if let Ok(Some(existing_sess)) = sessions.get_active(&key).await {
        let old_sid = existing_sess.get_session_id(&config.provider);
        if !old_sid.is_empty() {
            cli.sessions.terminate(&old_sid).await;
        }
    }
    let model = config.model.as_deref().unwrap_or("antigravity-default");
    let mut sess = sessions.reset_provider_session(&key, &config.provider, model).await.unwrap();
    let _ = crate::telegram::session_init::initialize_session_if_needed(bot, msg, sessions, &mut sess, cli, config).await;
    Ok(())
}

async fn handle_session_control_commands(
    bot: &Bot,
    msg: &Message,
    text: &str,
    config: &CliConfig,
    sessions: &crate::session::manager::SessionManager,
    cli: &AntigravityCli,
    topic_cache: &super::TopicNameCache,
) -> Result<bool, teloxide::RequestError> {
    let args = text.trim();
    let cmd = args.split_whitespace().next().unwrap_or("");
    let topic_id = crate::telegram::get_topic_id(msg);

    if cmd == "/new" || cmd == "/reset" {
        handle_new_command(bot, msg, args, topic_id, config, sessions, cli, topic_cache).await?;
        return Ok(true);
    }
    if cmd == "/stop" {
        let interrupted = cli.sessions.interrupt(msg.chat.id.0, topic_id).await;
        let reply_text = if interrupted {
            t!("bot.stop_interrupted")
        } else {
            t!("bot.stop_none")
        };
        let _ = send_reply(bot, msg, reply_text).await;
        return Ok(true);
    }
    if cmd == "/stop_all" || cmd == "/abort" {
        cli.sessions.terminate_all().await;
        let _ = send_reply(bot, msg, t!("bot.stop_all_success")).await;
        return Ok(true);
    }
    Ok(false)
}




async fn handle_memory_command(
    bot: &Bot,
    msg: &Message,
    config: &CliConfig,
) -> Result<(), teloxide::RequestError> {
    let memory_path = config.working_dir.join("memory_system/MAINMEMORY.md");
    let content = std::fs::read_to_string(memory_path)
        .unwrap_or_else(|_| t!("bot.memory_empty"));
    
    let html_text = crate::telegram::formatting::markdown_to_telegram_html(&content);
    let chunks = crate::telegram::formatting::split_html_message(&html_text, 4000);
    for chunk in chunks {
        let mut req = bot.send_message(msg.chat.id, chunk)
            .parse_mode(teloxide::types::ParseMode::Html);
        if let Some(tid) = msg.thread_id {
            req = req.message_thread_id(tid);
        }
        if let Err(e) = req.await {
            eprintln!("❌ [tuner] Failed to send memory chunk: {:?}", e);
        }
    }
    Ok(())
}

