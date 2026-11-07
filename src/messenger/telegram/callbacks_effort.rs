//! # Telegram Model, Effort, and Provider Callbacks
//!
//! ## Overview
//! Helper functions to handle Telegram callback queries for selecting CLI providers,
//! base models, and reasoning effort levels.
//!
//! ## Search Tags
//! #effort-callback, #model-selector, #provider-callback, #effort-level

use teloxide::prelude::*;
use crate::config::CliConfig;
use crate::session::manager::SessionManager;
use crate::cli::antigravity::AntigravityCli;

fn get_efforts_for_base(base: &str, models: &[String]) -> Vec<String> {
    let mut efforts = Vec::new();
    for m in models {
        if let Some(suffix) = m.strip_prefix(base) {
            for (suf, eff) in [
                ("-minimal", "minimal"),
                ("-low", "low"),
                ("-medium", "medium"),
                ("-high", "high"),
                ("-xhigh", "xhigh"),
                ("-max", "max"),
                ("-ultra", "ultra"),
            ] {
                if suffix == suf && !efforts.contains(&eff.to_string()) {
                    efforts.push(eff.to_string());
                }
            }
        }
    }
    efforts
}

async fn render_effort_keyboard(
    bot: &teloxide::Bot,
    msg: &Message,
    base: &str,
    efforts: &[String],
) -> Result<(), teloxide::RequestError> {
    let mut keyboard = Vec::new();
    let mut row = Vec::new();
    for eff in efforts {
        row.push(teloxide::types::InlineKeyboardButton::callback(
            eff.to_uppercase(),
            format!("model_effort:{}:{}", base, eff),
        ));
    }
    keyboard.push(row);
    let markup = teloxide::types::InlineKeyboardMarkup::new(keyboard);
    let prompt_html = super::formatting::markdown_to_telegram_html(&format!("Select reasoning effort level for `{}`:", base));
    let _ = bot.edit_message_text(msg.chat.id, msg.id, prompt_html)
        .parse_mode(teloxide::types::ParseMode::Html)
        .reply_markup(markup)
        .await;
    Ok(())
}

/// Executes `handle_model_base_callback` (handle model base callback).
pub async fn handle_model_base_callback(
    bot: &teloxide::Bot,
    msg: &Message,
    base: &str,
    sessions: &SessionManager,
    config: &CliConfig,
    cli: &AntigravityCli,
) {
    let key = crate::session::key::SessionKey::telegram(msg.chat.id.0, crate::telegram::get_topic_id(msg));
    let dm = config.model.as_deref().unwrap_or("antigravity-default");
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
        models = crate::telegram::commands_model::fallback_models_for_provider(&active_provider);
    }

    let efforts = get_efforts_for_base(base, &models);

    if efforts.is_empty() {
        if let Ok((mut s, _)) = sessions.resolve_session(&key, &config.provider, dm).await {
            s.model = base.to_string();
            s.effort = None;
            let _ = sessions.preserve_session_identity(&s).await;
            crate::messenger::telegram::commands_model::apply_model_to_active_pty(cli, &s).await;
            let html = super::formatting::markdown_to_telegram_html(&crate::t!("bot.model_switch_success", model = base));
            let _ = bot.edit_message_text(msg.chat.id, msg.id, html)
                .parse_mode(teloxide::types::ParseMode::Html)
                .await;
        }
    } else {
        let _ = render_effort_keyboard(bot, msg, base, &efforts).await;
    }
}

/// Executes `handle_model_effort_callback` (handle model effort callback).
pub async fn handle_model_effort_callback(
    bot: &teloxide::Bot,
    msg: &Message,
    base: &str,
    effort: &str,
    sessions: &SessionManager,
    config: &CliConfig,
    cli: &AntigravityCli,
) {
    let key = crate::session::key::SessionKey::telegram(msg.chat.id.0, crate::telegram::get_topic_id(msg));
    let dm = config.model.as_deref().unwrap_or("antigravity-default");
    if let Ok((mut s, _)) = sessions.resolve_session(&key, &config.provider, dm).await {
        s.model = base.to_string();
        s.effort = Some(effort.to_string());
        let _ = sessions.preserve_session_identity(&s).await;
        crate::messenger::telegram::commands_model::apply_model_to_active_pty(cli, &s).await;
        let display = format!("{} (effort: {})", base, effort);
        let html = super::formatting::markdown_to_telegram_html(&crate::t!("bot.model_switch_success", model = display));
        let _ = bot.edit_message_text(msg.chat.id, msg.id, html)
            .parse_mode(teloxide::types::ParseMode::Html)
            .await;
    }
}

/// Executes `handle_standalone_effort_callback` (handle standalone effort callback).
pub async fn handle_standalone_effort_callback(
    bot: &teloxide::Bot,
    msg: &Message,
    effort: &str,
    sessions: &SessionManager,
    config: &CliConfig,
    cli: &AntigravityCli,
) {
    let key = crate::session::key::SessionKey::telegram(msg.chat.id.0, crate::telegram::get_topic_id(msg));
    let dm = config.model.as_deref().unwrap_or("antigravity-default");
    if let Ok((mut s, _)) = sessions.resolve_session(&key, &config.provider, dm).await {
        s.effort = Some(effort.to_string());
        let _ = sessions.preserve_session_identity(&s).await;
        crate::messenger::telegram::commands_model::apply_model_to_active_pty(cli, &s).await;
        let display_msg = super::formatting::markdown_to_telegram_html(&format!("🤖 [tuner] Session reasoning effort switched to `{}`.", effort));
        let _ = bot.edit_message_text(msg.chat.id, msg.id, display_msg)
            .parse_mode(teloxide::types::ParseMode::Html)
            .await;
    }
}

fn build_provider_keyboard() -> teloxide::types::InlineKeyboardMarkup {
    use teloxide::types::InlineKeyboardButton as Btn;
    teloxide::types::InlineKeyboardMarkup::new(vec![
        vec![Btn::callback("🚀 Antigravity (agy)", "provider:antigravity")],
        vec![Btn::callback("🟠 Claude Code (claude)", "provider:claude")],
        vec![Btn::callback("🟢 OpenAI Codex (codex)", "provider:codex")],
    ])
}

/// Handles the `/provider` slash command and optional `<name>` argument.
pub async fn handle_provider_command(
    bot: &Bot,
    msg: &Message,
    args: &str,
    config: &CliConfig,
    sessions: &SessionManager,
) -> Result<(), teloxide::RequestError> {
    let key = crate::session::key::SessionKey::telegram(msg.chat.id.0, crate::telegram::get_topic_id(msg));
    let default_model = config.model.as_deref().unwrap_or("antigravity-default");
    let Ok((mut sess, _)) = sessions.resolve_session(&key, &config.provider, default_model).await else {
        return Ok(());
    };

    if args.is_empty() {
        let text = super::formatting::markdown_to_telegram_html(&format!(
            "Current CLI provider is `{}` (model: `{}`). Select a CLI provider:",
            sess.provider, sess.model
        ));
        let mut req = bot.send_message(msg.chat.id, text).parse_mode(teloxide::types::ParseMode::Html);
        if let Some(tid) = msg.thread_id {
            req = req.message_thread_id(tid);
        }
        let _ = req.reply_markup(build_provider_keyboard()).await;
    } else if let Some(target) = crate::session::data::SessionData::canonical_provider(args) {
        let reply_msg = sess.apply_provider_switch(target);
        let _ = sessions.preserve_session_identity(&sess).await;
        super::commands_registry::update_chat_commands_for_provider(bot, msg.chat.id, target).await;
        let _ = super::commands::send_reply(bot, msg, reply_msg).await;
    } else {
        let _ = super::commands::send_reply(
            bot,
            msg,
            "❌ Invalid provider. Supported providers: `antigravity` (`agy`), `claude`, `codex`.",
        )
        .await;
    }
    Ok(())
}

/// Handles inline keyboard callbacks (`provider:<name>`) for switching the active CLI provider.
pub async fn handle_provider_callback(
    bot: &teloxide::Bot,
    msg: &Message,
    raw_provider: &str,
    sessions: &SessionManager,
    config: &CliConfig,
) {
    let Some(target) = crate::session::data::SessionData::canonical_provider(raw_provider) else {
        return;
    };
    let key = crate::session::key::SessionKey::telegram(msg.chat.id.0, crate::telegram::get_topic_id(msg));
    let dm = config.model.as_deref().unwrap_or("antigravity-default");
    if let Ok((mut s, _)) = sessions.resolve_session(&key, &config.provider, dm).await {
        let status_text = s.apply_provider_switch(target);
        let _ = sessions.preserve_session_identity(&s).await;
        super::commands_registry::update_chat_commands_for_provider(bot, msg.chat.id, target).await;
        let html = super::formatting::markdown_to_telegram_html(&status_text);
        let _ = bot.edit_message_text(msg.chat.id, msg.id, html)
            .parse_mode(teloxide::types::ParseMode::Html)
            .await;
    }
}
