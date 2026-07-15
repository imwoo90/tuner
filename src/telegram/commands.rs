//! # Telegram Bot Commands
//!
//! This module handles commands sent to the Telegram bot, such as
//! /help, /status, /restart, /new, /abort, and /model.

use teloxide::prelude::*;
use crate::config::CliConfig;
use crate::cli::antigravity::AntigravityCli;
use crate::t;

async fn send_reply(
    bot: &Bot,
    msg: &Message,
    text: impl Into<String>,
) -> Result<Message, teloxide::RequestError> {
    let mut req = bot.send_message(msg.chat.id, text);
    if let Some(tid) = msg.thread_id {
        req = req.message_thread_id(tid);
    }
    req.await
}

async fn handle_info_commands(
    bot: &Bot,
    msg: &Message,
    text: &str,
    config: &CliConfig,
    sessions: &crate::session::manager::SessionManager,
) -> Result<bool, teloxide::RequestError> {
    if text == "/help" {
        let _ = send_reply(bot, msg, t!("bot.help")).await;
        return Ok(true);
    }
    if text == "/status" {
        let model_str = crate::telegram::reply::resolve_session_model(msg, config, sessions).await;
        let status_msg = t!("bot.status", provider = config.provider, model = model_str);
        let _ = send_reply(bot, msg, status_msg).await;
        return Ok(true);
    }
    if text == "/restart" {
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
    if handle_info_commands(bot, msg, text, config, sessions).await? {
        return Ok(true);
    }
    if text.starts_with("/new") || text.starts_with("/reset") || text == "/stop" || text == "/stop_all" || text == "/abort" {
        return handle_session_control_commands(bot, msg, text, config, sessions, cli, topic_cache).await;
    }
    if text.starts_with("/model") {
        let args = text["/model".len()..].trim();
        let _ = handle_model_command(bot, msg, args, config, sessions, cli).await;
        return Ok(true);
    }
    if text == "/diagnose" {
        let _ = handle_diagnose_command(bot, msg, config, sessions).await;
        return Ok(true);
    }
    if text == "/memory" {
        let _ = handle_memory_command(bot, msg).await;
        return Ok(true);
    }
    if text == "/cron" {
        let _ = handle_cron_command(bot, msg, cron_manager).await;
        return Ok(true);
    }
    Ok(false)
}

async fn handle_cron_command(
    bot: &Bot,
    msg: &Message,
    cron_manager: &crate::cron::manager::CronManager,
) -> Result<(), teloxide::RequestError> {
    if let Ok((txt, markup)) = crate::telegram::cron_selector::build_cron_page(cron_manager, 0, None).await {
        let mut req = bot.send_message(msg.chat.id, txt);
        if let Some(tid) = msg.thread_id {
            req = req.message_thread_id(tid);
        }
        let _ = req.reply_markup(markup).await;
    }
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
    let mut topic_id = crate::telegram::get_topic_id(msg);

    if cmd == "/new" || cmd == "/reset" {
        let parts: Vec<&str> = args.split_whitespace().collect();
        if parts.len() > 1 {
            let name = parts[1];
            if let Some(resolved_tid) = topic_cache.find_by_name(msg.chat.id.0, name) {
                topic_id = Some(resolved_tid);
            } else {
                let _ = send_reply(bot, msg, t!("bot.unknown_topic", name = name)).await;
                return Ok(true);
            }
        }
        let key = crate::session::key::SessionKey::telegram(msg.chat.id.0, topic_id);
        let model = config.model.as_deref().unwrap_or("antigravity-default");
        let _ = sessions.reset_provider_session(&key, &config.provider, model).await;
        let _ = send_reply(bot, msg, t!("bot.new_session")).await;
        return Ok(true);
    }
    if cmd == "/stop" {
        let count = cli.sessions.abort(msg.chat.id.0, topic_id).await;
        let _ = send_reply(bot, msg, t!("bot.stop_success", count = count)).await;
        return Ok(true);
    }
    if cmd == "/stop_all" || cmd == "/abort" {
        cli.sessions.terminate_all().await;
        let _ = send_reply(bot, msg, t!("bot.stop_all_success")).await;
        return Ok(true);
    }
    Ok(false)
}

async fn handle_model_command(
    bot: &Bot,
    msg: &Message,
    args: &str,
    config: &CliConfig,
    sessions: &crate::session::manager::SessionManager,
    cli: &AntigravityCli,
) -> Result<(), teloxide::RequestError> {
    let topic_id = crate::telegram::get_topic_id(msg);
    let key = crate::session::key::SessionKey::telegram(msg.chat.id.0, topic_id);
    if args.is_empty() {
        let mut models = cli.discover_models().await;
        if models.is_empty() {
            models = vec![
                "claude-3-5-sonnet".to_string(),
                "gemini-1.5-pro".to_string(),
                "antigravity-default".to_string(),
            ];
        }
        let mut keyboard = Vec::new();
        for m in &models {
            keyboard.push(vec![teloxide::types::InlineKeyboardButton::callback(m, format!("model:{}", m))]);
        }
        let markup = teloxide::types::InlineKeyboardMarkup::new(keyboard);
        let mut req = bot.send_message(msg.chat.id, t!("bot.model_select_header"));
        if let Some(tid) = msg.thread_id {
            req = req.message_thread_id(tid);
        }
        let _ = req.reply_markup(markup).await;
    } else {
        let default_model = config.model.as_deref().unwrap_or("antigravity-default");
        let (mut sess, _) = sessions.resolve_session(&key, &config.provider, default_model).await.unwrap();
        sess.model = args.to_string();
        let _ = sessions.update_session(&sess, 0.0, 0).await;
        
        let status_msg = t!("bot.model_switch_success", model = args);
        let _ = send_reply(bot, msg, status_msg).await;
    }
    Ok(())
}

async fn handle_diagnose_command(
    bot: &Bot,
    msg: &Message,
    config: &CliConfig,
    sessions: &crate::session::manager::SessionManager,
) -> Result<(), teloxide::RequestError> {
    let agy_status = match std::process::Command::new("agy").arg("--version").output() {
        Ok(out) => {
            let ver = String::from_utf8_lossy(&out.stdout).trim().to_string();
            t!("bot.diagnose_installed", version = ver)
        }
        Err(_) => t!("bot.diagnose_not_found"),
    };

    let session_count = sessions.load().map(|m| m.len()).unwrap_or(0);
    let token_present = if std::env::var("TELEGRAM_TOKEN").is_ok() || !config.telegram_token.is_empty() {
        t!("bot.diagnose_token_set")
    } else {
        t!("bot.diagnose_token_missing")
    };

    let model_str = crate::telegram::reply::resolve_session_model(msg, config, sessions).await;

    let report = t!(
        "bot.diagnose_report",
        agy_status = agy_status,
        token_present = token_present,
        session_count = session_count,
        provider = config.provider,
        model = model_str
    );

    let _ = send_reply(bot, msg, report).await;
    Ok(())
}

async fn handle_memory_command(
    bot: &Bot,
    msg: &Message,
) -> Result<(), teloxide::RequestError> {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/home/wimvm".to_string());
    let memory_path = std::path::PathBuf::from(home).join(".tuner/workspace/memory_system/MAINMEMORY.md");
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
        let _ = req.await;
    }
    Ok(())
}

pub(crate) fn get_bot_commands() -> Vec<teloxide::types::BotCommand> {
    let list = [
        ("help", "Show help and usage instructions"),
        ("new", "Start a fresh conversation session"),
        ("reset", "Alias for /new"),
        ("stop", "Cancel active CLI processes in chat"),
        ("abort", "Forcefully stop all running workers"),
        ("model", "Select or change active AI model"),
        ("status", "Show bot daemon and session metrics"),
        ("memory", "Print workspace MAINMEMORY.md contents"),
        ("diagnose", "Perform self-diagnostic validation checks"),
        ("restart", "Trigger clean restart of tuner service"),
        ("plan", "Request step-by-step plan before execution"),
        ("grill_me", "Start interactive interview alignment"),
        ("goal", "Launch long-running thorough task"),
        ("learn", "Record learning or behavior correction"),
        ("teamwork_preview", "Launch collaborative multi-agent simulation"),
    ];
    list.into_iter().map(|(c, d)| teloxide::types::BotCommand {
        command: c.to_string(),
        description: d.to_string(),
    }).collect()
}

pub(crate) async fn register_commands(bot: &Bot) -> Result<(), teloxide::RequestError> {
    bot.set_my_commands(get_bot_commands()).await?;
    Ok(())
}
