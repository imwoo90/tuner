//! # Telegram Model, Effort, and Provider Commands
//!
//! ## Overview
//! Handlers for Telegram slash commands `/model`, `/effort`, and `/provider` to manage active
//! CLI engine providers, LLM model selection, and reasoning effort levels.
//!
//! ## Search Tags
//! #model-command, #effort-command, #provider-command, #pty-model-switch

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
    let mut out: Vec<(String, Vec<String>)> = Vec::new();
    for m in raw_models {
        let (base, effort) = crate::cli::common::strip_effort_suffix(m);
        if let Some((_, effs)) = out.iter_mut().find(|(b, _)| b == base) {
            if let Some(eff) = effort.map(str::to_string) && !effs.contains(&eff) {
                effs.push(eff);
            }
        } else {
            let effs = effort.map(|e| vec![e.to_string()]).unwrap_or_default();
            out.push((base.to_string(), effs));
        }
    }
    out
}

fn normalize_base_model(m: &str) -> &str {
    AntigravityCli::normalize_model_id(m)
}

pub(crate) fn normalize_model_for_provider(provider: &str, m: &str) -> String {
    match provider {
        "claude" => crate::cli::claude::resolve_claude_model_effort(Some(m), None)
            .0
            .unwrap_or_else(|| m.to_string()),
        "codex" => crate::cli::codex::resolve_codex_model_effort(Some(m), None)
            .0
            .unwrap_or_else(|| m.to_string()),
        _ => normalize_base_model(m).to_string(),
    }
}

pub(crate) fn is_valid_effort_level(level: &str) -> bool {
    matches!(level, "minimal" | "low" | "medium" | "high" | "xhigh" | "max" | "ultra")
}

pub(crate) fn parse_model_effort_args(args: &str) -> (String, Option<String>) {
    let mut model_part = args.trim().to_string();
    let mut effort_part = None;

    if let Some(pos) = model_part.find("--effort") {
        let effort_val = model_part[pos + 8..].trim();
        effort_part = Some(effort_val.to_string());
        model_part = model_part[..pos].trim().to_string();
    } else {
        let parts: Vec<String> = model_part.split_whitespace().map(|s| s.to_string()).collect();
        if parts.len() == 2 && is_valid_effort_level(&parts[1]) {
            model_part = parts[0].clone();
            effort_part = Some(parts[1].clone());
        }
    }

    let (stripped, suf_eff) = crate::cli::common::strip_effort_suffix(&model_part);
    if let Some(eff) = suf_eff {
        model_part = stripped.to_string();
        effort_part = Some(eff.to_string());
    }

    let base = normalize_base_model(&model_part);
    if effort_part.is_none() && ["claude-", "opus", "sonnet", "pro", "argon"].iter().any(|k| base.contains(k)) {
        effort_part = Some("high".to_string());
    }

    (base.to_string(), effort_part)
}

pub(crate) fn fallback_models() -> Vec<String> {
    [
        "gemini-4-argon-high", "gemini-4-argon-medium", "gemini-4-argon-low",
        "gemini-3.8-flash-high", "gemini-3.8-flash-medium", "gemini-3.8-flash-low",
        "claude-opus-5-5-high", "claude-opus-5-5-medium", "claude-opus-5-5-low",
        "claude-sonnet-5-5-high", "claude-sonnet-5-5-medium", "claude-sonnet-5-5-low",
        "gpt-oss-120b-medium", "antigravity-default",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

pub(crate) fn fallback_models_for_provider(provider: &str) -> Vec<String> {
    match provider {
        "claude" => crate::cli::claude::discover_models(),
        "codex" => crate::cli::codex::discover_models(),
        _ => fallback_models(),
    }
}

async fn handle_model_command_empty(
    bot: &Bot,
    msg: &Message,
    config: &CliConfig,
    sessions: &crate::session::manager::SessionManager,
    cli: &AntigravityCli,
) -> Result<(), teloxide::RequestError> {
    let key = crate::session::key::SessionKey::telegram(msg.chat.id.0, crate::telegram::get_topic_id(msg));
    let active_provider = sessions
        .get_active(&key)
        .await
        .ok()
        .flatten()
        .map(|s| s.provider)
        .filter(|p| !p.is_empty())
        .unwrap_or_else(|| config.provider.clone());
    let mut p_cli = cli.clone();
    p_cli.config.provider = active_provider.clone();
    let mut models = p_cli.discover_models().await;
    if models.is_empty() {
        models = fallback_models_for_provider(&active_provider);
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
    sess: &crate::session::data::SessionData,
) {
    let sid = sess.get_session_id(&sess.provider);
    if sess.provider == "claude" {
        let _ = crate::cli::claude::set_model_live(sess.chat_id, sess.topic_id, Some(&sid), &sess.model).await;
        return;
    }
    if sess.provider == "codex" {
        return;
    }
    if !sid.is_empty() && cli.sessions.is_active(&sid).await {
        let _ = cli.ensure_interactive_ready(&sid).await;
        let target = if let Some(ref eff) = sess.effort {
            format!("{}-{}", sess.model, eff)
        } else {
            sess.model.clone()
        };
        let _ = cli.sessions.write_to_session(&sid, &format!("/model {}\r", target)).await;
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
    sess.model = normalize_model_for_provider(&sess.provider, &model_part);
    sess.effort = effort_part.clone();
    let _ = sessions.preserve_session_identity(&sess).await;

    apply_model_to_active_pty(cli, &sess).await;

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
        handle_model_command_empty(bot, msg, config, sessions, cli).await
    } else {
        handle_model_command_switch(bot, msg, args, config, sessions, cli).await
    }
}

pub(crate) fn build_effort_keyboard_for_provider(provider: &str) -> teloxide::types::InlineKeyboardMarkup {
    let levels: &[(&str, &str)] = match provider {
        "claude" => &[("Low", "low"), ("Medium", "medium"), ("High", "high"), ("XHigh", "xhigh"), ("Max", "max")],
        "codex" => &[("Minimal", "minimal"), ("Low", "low"), ("Medium", "medium"), ("High", "high"), ("XHigh", "xhigh")],
        _ => &[("High", "high"), ("Medium", "medium"), ("Low", "low")],
    };
    let row: Vec<_> = levels
        .iter()
        .map(|(lbl, val)| teloxide::types::InlineKeyboardButton::callback(*lbl, format!("effort:{}", val)))
        .collect();
    teloxide::types::InlineKeyboardMarkup::new(vec![row])
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
        let _ = req.reply_markup(build_effort_keyboard_for_provider(&sess.provider)).await;
    } else {
        let level = args.trim().to_lowercase();
        if is_valid_effort_level(&level) {
            sess.effort = Some(level.clone());
            let _ = sessions.preserve_session_identity(&sess).await;
            apply_model_to_active_pty(cli, &sess).await;
            let _ = send_reply(bot, msg, format!("🤖 [tuner] Session reasoning effort switched to `{level}`.")).await;
        } else {
            let _ = send_reply(
                bot,
                msg,
                "❌ Invalid effort level. Supported levels: `minimal`, `low`, `medium`, `high`, `xhigh`, `max`.",
            )
            .await;
        }
    }
    Ok(())
}
