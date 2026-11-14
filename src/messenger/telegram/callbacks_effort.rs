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
use crate::session::{data::SessionData, key::SessionKey, manager::SessionManager};
use crate::cli::{antigravity::AntigravityCli, common::strip_effort_suffix};
use std::collections::HashMap;

fn count_efforts_per_base(raw_models: &[String]) -> HashMap<String, usize> {
    let mut counts: HashMap<String, Vec<String>> = HashMap::new();
    for m in raw_models {
        let (base, eff_opt) = strip_effort_suffix(m);
        if let Some(eff) = eff_opt {
            let list = counts.entry(base.to_string()).or_default();
            if !list.iter().any(|e| e == eff) {
                list.push(eff.to_string());
            }
        }
    }
    counts.into_iter().map(|(b, v)| (b, v.len())).collect()
}

pub(crate) fn group_discovered_models(raw_models: &[String]) -> Vec<(String, Vec<String>)> {
    let counts = count_efforts_per_base(raw_models);
    let mut out: Vec<(String, Vec<String>)> = Vec::new();
    for m in raw_models {
        let (base, effort) = strip_effort_suffix(m);
        if counts.get(base).copied().unwrap_or(0) >= 2 {
            if let Some((_, effs)) = out.iter_mut().find(|(b, _)| b == base) {
                if let Some(eff) = effort.map(str::to_string) && !effs.contains(&eff) {
                    effs.push(eff);
                }
            } else {
                out.push((base.to_string(), effort.map(|e| vec![e.to_string()]).unwrap_or_default()));
            }
        } else if !out.iter().any(|(b, _)| b == m) {
            out.push((m.clone(), Vec::new()));
        }
    }
    out
}

pub(crate) async fn discover_models_with_default(cli: &AntigravityCli, provider: &str) -> Vec<String> {
    let mut p_cli = cli.clone();
    p_cli.config.provider = provider.to_string();
    let mut models = p_cli.discover_models().await;
    if models.is_empty() {
        models = match provider {
            "claude" => crate::cli::claude::discover_models(),
            "codex" => crate::cli::codex::discover_models(),
            _ => crate::cli::antigravity::model_spec::fallback_models(),
        };
    }
    if !matches!(provider, "claude" | "codex") && !models.iter().any(|m| m == "antigravity-default") {
        models.push("antigravity-default".to_string());
    }
    models
}

fn get_efforts_for_base(base: &str, models: &[String]) -> Vec<String> {
    if models.iter().any(|m| m == base) {
        return Vec::new();
    }
    let mut efforts = Vec::new();
    for m in models {
        let (b, eff_opt) = strip_effort_suffix(m);
        if b == base
            && let Some(eff) = eff_opt
            && !efforts.iter().any(|e| e == eff)
        {
            efforts.push(eff.to_string());
        }
    }
    if efforts.len() < 2 { Vec::new() } else { efforts }
}

async fn render_effort_keyboard(
    bot: &teloxide::Bot,
    msg: &Message,
    base: &str,
    efforts: &[String],
) -> Result<(), teloxide::RequestError> {
    let row: Vec<_> = efforts
        .iter()
        .map(|eff| teloxide::types::InlineKeyboardButton::callback(eff.to_uppercase(), format!("model_effort:{base}:{eff}")))
        .collect();
    let markup = teloxide::types::InlineKeyboardMarkup::new(vec![row]);
    let prompt_html = super::formatting::markdown_to_telegram_html(&format!("Select reasoning effort level for `{base}`:"));
    let _ = bot.edit_message_text(msg.chat.id, msg.id, prompt_html)
        .parse_mode(teloxide::types::ParseMode::Html)
        .reply_markup(markup)
        .await;
    Ok(())
}

pub(crate) fn is_model_in_provider_catalog(base: &str, models: &[String]) -> bool {
    models.iter().any(|m| m == base || strip_effort_suffix(m).0 == base)
}

async fn validate_callback_provider_catalog(
    bot: &teloxide::Bot,
    msg: &Message,
    base: &str,
    key: &SessionKey,
    sessions: &SessionManager,
    config: &CliConfig,
    cli: &AntigravityCli,
) -> Option<(String, Vec<String>)> {
    let raw_prov = sessions
        .get_active(key)
        .await
        .ok()
        .flatten()
        .map(|s| s.provider)
        .unwrap_or_default();
    let active_provider = SessionData::effective_provider(&raw_prov, &config.provider).to_string();
    let models = discover_models_with_default(cli, &active_provider).await;
    if !is_model_in_provider_catalog(base, &models) {
        edit_markdown_status(
            bot,
            msg,
            &format!("⚠️ Model `{base}` is not available for active provider `{active_provider}`. Run `/model` to refresh."),
        )
        .await;
        return None;
    }
    Some((active_provider, models))
}

async fn apply_callback_model_selection(
    bot: &teloxide::Bot,
    msg: &Message,
    key: &SessionKey,
    selection: (&str, &str, Option<&str>),
    sessions: &SessionManager,
    config: &CliConfig,
    cli: &AntigravityCli,
) {
    let (active_provider, base, raw_effort) = selection;
    let dm = config.model.as_deref().unwrap_or("");
    let Ok((mut s, _)) = sessions.get_or_init_session(key, &config.provider, dm).await else { return; };
    let mut changed_prov = None;
    if let Ok(Some(u)) = sessions.mutate_session(key, Some(&s), |cur| {
        let cp = SessionData::effective_provider(&cur.provider, &config.provider);
        if cp != active_provider { changed_prov = Some(cp.to_string()); return false; }
        cur.provider = cp.to_string();
        cur.model = base.to_string();
        cur.effort = raw_effort.and_then(|e| super::reply_directives::clamp_session_effort(cp, base, e));
        true
    }).await { s = u; }
    if let Some(cp) = changed_prov {
        edit_markdown_status(bot, msg, &format!("⚠️ Active provider changed to `{cp}`. Run `/model` to refresh.")).await;
        return;
    }
    super::commands_model::apply_model_to_active_pty(cli, sessions, &s).await;
    let display = match s.effort {
        Some(ref e) => format!("{base} (effort: {e})"),
        None => base.to_string(),
    };
    edit_markdown_status(bot, msg, &crate::t!("bot.model_switch_success", model = display)).await;
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
    let key = SessionKey::telegram(msg.chat.id.0, crate::telegram::get_topic_id(msg));
    let Some((active_provider, models)) =
        validate_callback_provider_catalog(bot, msg, base, &key, sessions, config, cli).await
    else {
        return;
    };
    let efforts = get_efforts_for_base(base, &models);
    if efforts.is_empty() {
        apply_callback_model_selection(bot, msg, &key, (&active_provider, base, None), sessions, config, cli).await;
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
    let low = effort.to_ascii_lowercase();
    if !super::commands_model::is_valid_effort_level(&low) {
        return;
    }
    let key = SessionKey::telegram(msg.chat.id.0, crate::telegram::get_topic_id(msg));
    let Some((active_provider, models)) =
        validate_callback_provider_catalog(bot, msg, base, &key, sessions, config, cli).await
    else {
        return;
    };
    if !get_efforts_for_base(base, &models).iter().any(|e| e == &low) {
        edit_markdown_status(bot, msg, &format!("⚠️ Effort `{low}` is not available for `{base}`. Run `/model` to refresh.")).await;
        return;
    }
    apply_callback_model_selection(bot, msg, &key, (&active_provider, base, Some(effort)), sessions, config, cli).await;
}

pub(crate) async fn edit_markdown_status(bot: &teloxide::Bot, msg: &Message, md: &str) {
    let html = super::formatting::markdown_to_telegram_html(md);
    let _ = bot.edit_message_text(msg.chat.id, msg.id, html)
        .parse_mode(teloxide::types::ParseMode::Html)
        .await;
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
    let key = SessionKey::telegram(msg.chat.id.0, crate::telegram::get_topic_id(msg));
    let raw_prov = sessions.get_active(&key).await.ok().flatten().map(|s| s.provider).unwrap_or_default();
    if !matches!(SessionData::effective_provider(&raw_prov, &config.provider), "claude" | "codex") {
        crate::cli::antigravity::model_spec::ensure_live_catalog_fresh().await;
    }
    let dm = config.model.as_deref().unwrap_or("");
    let Ok((mut s, _)) = sessions.get_or_init_session(&key, &config.provider, dm).await else { return; };
    let mut outcome = Err(None);
    if let Ok(Some(u)) = sessions.mutate_session(&key, Some(&s), |cur| {
        let (cp, cm) = super::commands_model::eff_prov_model(cur, config);
        outcome = super::reply::validate_standalone_effort_payload(effort, &cp, &cm);
        match &outcome {
            Ok(resolved) => {
                cur.provider = cp;
                cur.effort = Some(resolved.clone());
                true
            }
            Err(Some(_)) if !super::reply_directives::session_supports_effort(&cp, &cm) && cur.effort.is_some() => {
                cur.effort = None;
                true
            }
            _ => false,
        }
    }).await { s = u; }
    match outcome {
        Ok(resolved) => {
            super::commands_model::apply_model_to_active_pty(cli, sessions, &s).await;
            edit_markdown_status(bot, msg, &format!("🤖 [tuner] Session reasoning effort switched to `{resolved}`.")).await;
        }
        Err(Some(warn)) => edit_markdown_status(bot, msg, &warn).await,
        Err(None) => {}
    }
}
