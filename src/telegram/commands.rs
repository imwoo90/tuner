//! # Telegram Bot Commands
//!
//! This module handles commands sent to the Telegram bot, such as
//! /help, /status, /restart, /new, /abort, and /model.

use teloxide::prelude::*;
use crate::config::CliConfig;
use crate::cli::antigravity::AntigravityCli;

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
) -> Result<bool, teloxide::RequestError> {
    if text == "/help" {
        let _ = send_reply(bot, msg, "🤖 [우덕터] 도움말:\n- 일반 메시지 송신 시 agy CLI 에이전트와 대화합니다.").await;
        return Ok(true);
    }
    if text == "/status" {
        let status_msg = format!(
            "🤖 [우덕터] 상태:\n- 프로바이더: {}\n- 모델: {:?}",
            config.provider, config.model
        );
        let _ = send_reply(bot, msg, status_msg).await;
        return Ok(true);
    }
    if text == "/restart" {
        let _ = send_reply(bot, msg, "🤖 [우덕터] 재기동을 요청합니다...").await;
        let home = std::env::var("HOME").unwrap_or_else(|_| "/home/wimvm".to_string());
        let restart_path = std::path::PathBuf::from(home).join(".ductor").join("restart-sentinel.json");
        let _ = std::fs::write(restart_path, "");
        return Ok(true);
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
    if handle_info_commands(bot, msg, text, config).await? {
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
                let _ = send_reply(bot, msg, format!("⚠️ [우덕터] 알 수 없는 토픽 이름입니다: {}", name)).await;
                return Ok(true);
            }
        }
        let key = crate::session::key::SessionKey::telegram(msg.chat.id.0, topic_id);
        let model = config.model.as_deref().unwrap_or("antigravity-default");
        let _ = sessions.reset_provider_session(&key, &config.provider, model).await;
        let _ = send_reply(bot, msg, "🤖 [우덕터] 기존 세션을 초기화하고 새 대화를 시작합니다.").await;
        return Ok(true);
    }
    if cmd == "/stop" {
        let count = cli.sessions.abort(msg.chat.id.0, topic_id).await;
        let _ = send_reply(bot, msg, format!("🤖 [우덕터] 이 토픽에서 진행 중인 프로세스 {}개를 강제 종료(stop)했습니다.", count)).await;
        return Ok(true);
    }
    if cmd == "/stop_all" || cmd == "/abort" {
        cli.sessions.terminate_all().await;
        let _ = send_reply(bot, msg, "🤖 [우덕터] 진행 중인 모든 프로세스를 강제 종료(abort)했습니다.").await;
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
        let mut req = bot.send_message(msg.chat.id, "🤖 [우덕터] 사용할 모델을 아래에서 선택해 주세요:");
        if let Some(tid) = msg.thread_id {
            req = req.message_thread_id(tid);
        }
        let _ = req.reply_markup(markup).await;
    } else {
        let default_model = config.model.as_deref().unwrap_or("antigravity-default");
        let (mut sess, _) = sessions.resolve_session(&key, &config.provider, default_model).await.unwrap();
        sess.model = args.to_string();
        let _ = sessions.update_session(&sess, 0.0, 0).await;
        
        let status_msg = format!("🤖 [우덕터] 세션의 LLM 모델이 `{}`(으)로 전환되었습니다.", args);
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
            format!("🟢 Installed ({})", ver)
        }
        Err(_) => "🔴 Not found in PATH".to_string(),
    };

    let session_count = sessions.load().map(|m| m.len()).unwrap_or(0);
    let token_present = if std::env::var("TELEGRAM_TOKEN").is_ok() || !config.telegram_token.is_empty() {
        "🟢 Set"
    } else {
        "🔴 Missing"
    };

    let report = format!(
        "🤖 [우덕터] 자가 진단 리포트\n\n\
         - agy CLI 상태: {}\n\
         - 텔레그램 토큰: {}\n\
         - 활성 세션 수: {} 개\n\
         - 프로바이더: {}\n\
         - 기본 모델: {:?}",
        agy_status, token_present, session_count, config.provider, config.model
    );

    let _ = send_reply(bot, msg, report).await;
    Ok(())
}

async fn handle_memory_command(
    bot: &Bot,
    msg: &Message,
) -> Result<(), teloxide::RequestError> {
    let content = std::fs::read_to_string("/home/wimvm/ductor/workspace/memory_system/MAINMEMORY.md")
        .unwrap_or_else(|_| "🤖 [우덕터] 현재 등록된 기억(MAINMEMORY.md)이 없습니다.".to_string());
    
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
