//! # Telegram Model and Effort Callbacks
//!
//! Helper functions to handle Telegram callback queries for selecting base models
//! and reasoning effort levels.

use teloxide::prelude::*;
use crate::config::CliConfig;
use crate::session::manager::SessionManager;
use crate::cli::antigravity::AntigravityCli;

fn get_efforts_for_base(base: &str, models: &[String]) -> Vec<String> {
    let mut efforts = Vec::new();
    for m in models {
        if m.starts_with(base) {
            let suffix = &m[base.len()..];
            if suffix == "-high" { efforts.push("high".to_string()); }
            else if suffix == "-medium" { efforts.push("medium".to_string()); }
            else if suffix == "-low" { efforts.push("low".to_string()); }
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
    let _ = bot.edit_message_text(msg.chat.id, msg.id, format!("Select reasoning effort level for `{}`:", base))
        .reply_markup(markup)
        .await;
    Ok(())
}

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
    
    let mut models = cli.discover_models().await;
    if models.is_empty() {
        models = vec![
            "gemini-3.6-flash-high".to_string(),
            "gemini-3.6-flash-medium".to_string(),
            "gemini-3.6-flash-low".to_string(),
            "gemini-3.5-flash-high".to_string(),
            "gemini-3.1-pro-high".to_string(),
            "claude-sonnet-4-6".to_string(),
            "antigravity-default".to_string(),
        ];
    }
    
    let efforts = get_efforts_for_base(base, &models);
    
    if efforts.is_empty() {
        if let Ok((mut s, _)) = sessions.resolve_session(&key, &config.provider, dm).await {
            s.model = base.to_string();
            s.effort = None;
            let _ = sessions.update_session(&s, 0.0, 0).await;
            let _ = bot.edit_message_text(msg.chat.id, msg.id, crate::t!("bot.model_switch_success", model = base)).await;
        }
    } else {
        let _ = render_effort_keyboard(bot, msg, base, &efforts).await;
    }
}

pub async fn handle_model_effort_callback(
    bot: &teloxide::Bot,
    msg: &Message,
    base: &str,
    effort: &str,
    sessions: &SessionManager,
    config: &CliConfig,
) {
    let key = crate::session::key::SessionKey::telegram(msg.chat.id.0, crate::telegram::get_topic_id(msg));
    let dm = config.model.as_deref().unwrap_or("antigravity-default");
    if let Ok((mut s, _)) = sessions.resolve_session(&key, &config.provider, dm).await {
        s.model = base.to_string();
        s.effort = Some(effort.to_string());
        let _ = sessions.update_session(&s, 0.0, 0).await;
        let display = format!("{} (effort: {})", base, effort);
        let _ = bot.edit_message_text(msg.chat.id, msg.id, crate::t!("bot.model_switch_success", model = display)).await;
    }
}

pub async fn handle_standalone_effort_callback(
    bot: &teloxide::Bot,
    msg: &Message,
    effort: &str,
    sessions: &SessionManager,
    config: &CliConfig,
) {
    let key = crate::session::key::SessionKey::telegram(msg.chat.id.0, crate::telegram::get_topic_id(msg));
    let dm = config.model.as_deref().unwrap_or("antigravity-default");
    if let Ok((mut s, _)) = sessions.resolve_session(&key, &config.provider, dm).await {
        s.effort = Some(effort.to_string());
        let _ = sessions.update_session(&s, 0.0, 0).await;
        let display_msg = format!("🤖 [tuner] Session reasoning effort switched to `{}`.", effort);
        let _ = bot.edit_message_text(msg.chat.id, msg.id, display_msg).await;
    }
}
