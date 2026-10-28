//! # Telegram Model and Effort Commands
//!
//! ## Overview
//! Handlers for Telegram slash commands `/model` and `/effort` to manage active LLM model selection
//! and reasoning effort.
//!
//! ## Search Tags
//! #model-command, #effort-command, #pty-model-switch

use teloxide::prelude::*;
use crate::config::CliConfig;
use crate::cli::antigravity::AntigravityCli;
use crate::t;

async fn send_reply(
    bot: &Bot,
    msg: &Message,
    text: impl Into<String>,
) -> Result<Message, teloxide::RequestError> {
    super::commands::send_reply(bot, msg, text).await
}

fn group_discovered_models(raw_models: &[String]) -> Vec<(String, Vec<String>)> {
    let mut map: std::collections::BTreeMap<String, Vec<String>> = std::collections::BTreeMap::new();
    for m in raw_models {
        let (base, effort) = if m.ends_with("-high") {
            (m[..m.len() - 5].to_string(), Some("high".to_string()))
        } else if m.ends_with("-medium") {
            (m[..m.len() - 7].to_string(), Some("medium".to_string()))
        } else if m.ends_with("-low") {
            (m[..m.len() - 4].to_string(), Some("low".to_string()))
        } else {
            (m.clone(), None)
        };
        
        let entry = map.entry(base).or_default();
        if let Some(eff) = effort
            && !entry.contains(&eff) {
                entry.push(eff);
            }
    }
    map.into_iter().collect()
}

fn normalize_base_model(m: &str) -> &str {
    AntigravityCli::normalize_model_id(m)
}

fn parse_model_effort_args(args: &str) -> (String, Option<String>) {
    let mut model_part = args.trim().to_string();
    let mut effort_part = None;

    if let Some(pos) = model_part.find("--effort") {
        let effort_val = model_part[pos + 8..].trim();
        effort_part = Some(effort_val.to_string());
        model_part = model_part[..pos].trim().to_string();
    } else {
        let parts: Vec<String> = model_part.split_whitespace().map(|s| s.to_string()).collect();
        if parts.len() == 2 && (parts[1] == "high" || parts[1] == "medium" || parts[1] == "low") {
            model_part = parts[0].clone();
            effort_part = Some(parts[1].clone());
        }
    }

    for (suffix, eff) in [("-high", "high"), ("-medium", "medium"), ("-low", "low")] {
        if model_part.ends_with(suffix) {
            effort_part = Some(eff.to_string());
            model_part.truncate(model_part.len() - suffix.len());
            break;
        }
    }

    let base = normalize_base_model(&model_part);
    if effort_part.is_none() && ["claude-", "opus", "sonnet", "pro", "argon"].iter().any(|k| base.contains(k)) {
        effort_part = Some("high".to_string());
    }

    (base.to_string(), effort_part)
}

pub(crate) fn fallback_models() -> Vec<String> {
    vec![
        "gemini-4-argon-high".to_string(),
        "gemini-4-argon-medium".to_string(),
        "gemini-4-argon-low".to_string(),
        "gemini-3.8-flash-high".to_string(),
        "gemini-3.8-flash-medium".to_string(),
        "gemini-3.8-flash-low".to_string(),
        "claude-opus-5-5-high".to_string(),
        "claude-opus-5-5-medium".to_string(),
        "claude-opus-5-5-low".to_string(),
        "claude-sonnet-5-5-high".to_string(),
        "claude-sonnet-5-5-medium".to_string(),
        "claude-sonnet-5-5-low".to_string(),
        "gpt-oss-120b-medium".to_string(),
        "antigravity-default".to_string(),
    ]
}

async fn handle_model_command_empty(
    bot: &Bot,
    msg: &Message,
    cli: &AntigravityCli,
) -> Result<(), teloxide::RequestError> {
    let mut models = cli.discover_models().await;
    if models.is_empty() {
        models = fallback_models();
    }
    
    let grouped = group_discovered_models(&models);
    let mut keyboard = Vec::new();
    for (base_name, _efforts) in &grouped {
        keyboard.push(vec![teloxide::types::InlineKeyboardButton::callback(
            base_name,
            format!("model_base:{}", base_name),
        )]);
    }
    let markup = teloxide::types::InlineKeyboardMarkup::new(keyboard);
    let mut req = bot.send_message(msg.chat.id, t!("bot.model_select_header"));
    if let Some(tid) = msg.thread_id {
        req = req.message_thread_id(tid);
    }
    if let Err(e) = req.reply_markup(markup).await {
        eprintln!("❌ [tuner] Failed to send model select keyboard: {:?}", e);
    }
    Ok(())
}

pub(crate) async fn apply_model_to_active_pty(
    cli: &AntigravityCli,
    sid: &str,
    model: &str,
    effort: Option<&str>,
) {
    if !sid.is_empty() && cli.sessions.is_active(sid).await {
        let _ = cli.ensure_interactive_ready(sid).await;
        let target = if let Some(eff) = effort {
            format!("{}-{}", model, eff)
        } else {
            model.to_string()
        };
        let _ = cli.sessions.write_to_session(sid, &format!("/model {}\r", target)).await;
        tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
    }
}

async fn handle_model_command_switch(
    bot: &Bot,
    msg: &Message,
    args: &str,
    config: &CliConfig,
    sessions: &crate::session::manager::SessionManager,
    cli: &AntigravityCli,
) -> Result<(), teloxide::RequestError> {
    let topic_id = crate::telegram::get_topic_id(msg);
    let key = crate::session::key::SessionKey::telegram(msg.chat.id.0, topic_id);
    let default_model = config.model.as_deref().unwrap_or("antigravity-default");
    let Ok((mut sess, _)) = sessions.resolve_session(&key, &config.provider, default_model).await else {
        return Ok(());
    };
    
    let (model_part, effort_part) = parse_model_effort_args(args);
    sess.model = model_part;
    sess.effort = effort_part.clone();
    let _ = sessions.preserve_session_identity(&sess).await;
    
    let sid = sess.get_session_id(&sess.provider);
    apply_model_to_active_pty(cli, &sid, &sess.model, sess.effort.as_deref()).await;

    let display_model = if let Some(ref eff) = effort_part {
        format!("{} (effort: {})", sess.model, eff)
    } else {
        sess.model.clone()
    };
    let status_msg = t!("bot.model_switch_success", model = display_model);
    let _ = send_reply(bot, msg, status_msg).await;
    Ok(())
}

pub(crate) async fn handle_model_command(
    bot: &Bot,
    msg: &Message,
    args: &str,
    config: &CliConfig,
    sessions: &crate::session::manager::SessionManager,
    cli: &AntigravityCli,
) -> Result<(), teloxide::RequestError> {
    if args.is_empty() {
        handle_model_command_empty(bot, msg, cli).await
    } else {
        handle_model_command_switch(bot, msg, args, config, sessions, cli).await
    }
}

fn build_effort_keyboard() -> teloxide::types::InlineKeyboardMarkup {
    teloxide::types::InlineKeyboardMarkup::new(vec![vec![
        teloxide::types::InlineKeyboardButton::callback("High", "effort:high"),
        teloxide::types::InlineKeyboardButton::callback("Medium", "effort:medium"),
        teloxide::types::InlineKeyboardButton::callback("Low", "effort:low"),
    ]])
}

pub(crate) async fn handle_effort_command(
    bot: &Bot,
    msg: &Message,
    args: &str,
    config: &CliConfig,
    sessions: &crate::session::manager::SessionManager,
    cli: &AntigravityCli,
) -> Result<(), teloxide::RequestError> {
    let topic_id = crate::telegram::get_topic_id(msg);
    let key = crate::session::key::SessionKey::telegram(msg.chat.id.0, topic_id);
    let default_model = config.model.as_deref().unwrap_or("antigravity-default");
    let Ok((mut sess, _)) = sessions.resolve_session(&key, &config.provider, default_model).await else {
        return Ok(());
    };
    
    if args.is_empty() {
        let cur = sess.effort.as_deref().unwrap_or("default");
        let text = super::formatting::markdown_to_telegram_html(&format!(
            "Current reasoning effort is `{cur}`. Select a new reasoning effort level:"
        ));
        let mut req = bot.send_message(msg.chat.id, text).parse_mode(teloxide::types::ParseMode::Html);
        if let Some(tid) = msg.thread_id {
            req = req.message_thread_id(tid);
        }
        let _ = req.reply_markup(build_effort_keyboard()).await;
    } else {
        let level = args.trim().to_lowercase();
        if matches!(level.as_str(), "high" | "medium" | "low") {
            sess.effort = Some(level.clone());
            let _ = sessions.preserve_session_identity(&sess).await;
            let sid = sess.get_session_id(&sess.provider);
            apply_model_to_active_pty(cli, &sid, &sess.model, sess.effort.as_deref()).await;
            let _ = send_reply(bot, msg, format!("🤖 [tuner] Session reasoning effort switched to `{level}`.")).await;
        } else {
            let _ = send_reply(bot, msg, "❌ Invalid effort level. Supported levels: `high`, `medium`, `low`.").await;
        }
    }
    Ok(())
}
