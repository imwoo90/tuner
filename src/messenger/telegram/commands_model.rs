//! # Telegram Model, Effort, and Provider Commands
//!
//! ## Overview
//! Handlers for Telegram slash commands `/model` and `/effort` to manage active LLM model
//! selection, reasoning effort clamping, and live PTY `/model` synchronization.
//!
//! ## Search Tags
//! #model-command, #effort-command, #provider-command, #pty-model-switch

use teloxide::prelude::*;
use crate::config::CliConfig;
use crate::cli::antigravity::AntigravityCli;
use crate::session::{data::SessionData, key::SessionKey, manager::SessionManager};
use crate::t;
use super::commands::send_reply;

pub(crate) use super::callbacks_effort::{discover_models_with_default, group_discovered_models};

pub(crate) fn normalize_model_for_provider(provider: &str, m: &str) -> String {
    match SessionData::canonical_provider(provider).unwrap_or("antigravity") {
        "claude" => crate::cli::claude::resolve_claude_model_effort(Some(m), None)
            .0
            .unwrap_or_else(|| m.to_string()),
        "codex" => crate::cli::codex::resolve_codex_model_effort(Some(m), None)
            .0
            .unwrap_or_else(|| m.to_string()),
        _ => crate::cli::antigravity::model_spec::canonicalize_model(m),
    }
}

pub(crate) fn is_valid_effort_level(level: &str) -> bool {
    matches!(level, "minimal" | "low" | "medium" | "high" | "xhigh" | "max" | "ultra")
}

pub(crate) use super::reply_directives::{
    clamp_session_effort, parse_model_effort_args_for_provider,
    session_supports_effort, tokenize_model_command_args,
};

async fn handle_model_command_empty(
    bot: &Bot,
    msg: &Message,
    config: &CliConfig,
    sessions: &SessionManager,
    cli: &AntigravityCli,
) -> Result<(), teloxide::RequestError> {
    let key = SessionKey::telegram(msg.chat.id.0, crate::telegram::get_topic_id(msg));
    let raw_prov = sessions
        .get_active(&key)
        .await
        .ok()
        .flatten()
        .map(|s| s.provider)
        .unwrap_or_default();
    let active_provider = SessionData::effective_provider(&raw_prov, &config.provider).to_string();
    let models = discover_models_with_default(cli, &active_provider).await;

    let grouped = group_discovered_models(&models);
    let mut keyboard = Vec::new();
    for (base_name, _efforts) in &grouped {
        keyboard.push(vec![teloxide::types::InlineKeyboardButton::callback(
            base_name,
            format!("model_base:{base_name}"),
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

pub(crate) use super::commands_provider::apply_model_to_active_pty;

async fn persist_model_command_switch(
    key: &SessionKey,
    sess: &mut SessionData,
    target: (&str, &str, Option<&str>),
    config: &CliConfig,
    sessions: &SessionManager,
) -> Result<(), String> {
    let (eff_prov, canonical, effort_opt) = target;
    if !SessionData::is_model_compatible(canonical, eff_prov) {
        return Err(eff_prov.to_string());
    }
    let mut mismatch_prov = None;
    if let Ok(Some(updated)) = sessions
        .mutate_session(key, Some(sess), |cur| {
            let cp = SessionData::effective_provider(&cur.provider, &config.provider).to_string();
            if !SessionData::is_model_compatible(canonical, &cp) {
                mismatch_prov = Some(cp);
                return false;
            }
            cur.provider = cp;
            cur.model = canonical.to_string();
            cur.effort = effort_opt.map(str::to_string);
            true
        })
        .await
    {
        *sess = updated;
    } else {
        sess.provider = eff_prov.to_string();
        sess.model = canonical.to_string();
        sess.effort = effort_opt.map(str::to_string);
    }
    match mismatch_prov {
        Some(cp) => Err(cp),
        None => Ok(()),
    }
}

async fn handle_model_command_switch(
    bot: &Bot,
    msg: &Message,
    args: &str,
    config: &CliConfig,
    sessions: &SessionManager,
    cli: &AntigravityCli,
) -> Result<(), teloxide::RequestError> {
    if tokenize_model_command_args(args).is_err() {
        let _ = send_reply(bot, msg, "❌ Invalid effort level. Supported levels: `minimal`, `low`, `medium`, `high`, `xhigh`, `max`.").await;
        return Ok(());
    }
    let key = SessionKey::telegram(msg.chat.id.0, crate::telegram::get_topic_id(msg));
    let raw_prov = sessions.get_active(&key).await.ok().flatten().map(|s| s.provider).unwrap_or_default();
    if !matches!(SessionData::effective_provider(&raw_prov, &config.provider), "claude" | "codex") {
        crate::cli::antigravity::model_spec::ensure_live_catalog_fresh().await;
    }
    let dm = config.model.as_deref().unwrap_or("");
    let Ok((mut sess, _)) = sessions.get_or_init_session(&key, &config.provider, dm).await else { return Ok(()); };
    let eff_prov = SessionData::effective_provider(&sess.provider, &config.provider).to_string();
    let cfg_eff = SessionData::configured_effort_fallback(config, &eff_prov);
    let (canonical, effort_part) = parse_model_effort_args_for_provider(&eff_prov, args, cfg_eff.as_deref());
    if let Err(cp) = persist_model_command_switch(&key, &mut sess, (&eff_prov, &canonical, effort_part.as_deref()), config, sessions).await {
        let _ = send_reply(bot, msg, format!("⚠️ Model `{canonical}` is not available for active provider `{cp}`.")).await;
        return Ok(());
    }
    apply_model_to_active_pty(cli, sessions, &sess).await;
    let display = match effort_part {
        Some(ref eff) => format!("{} (effort: {eff})", sess.model),
        None => sess.model.clone(),
    };
    let _ = send_reply(bot, msg, t!("bot.model_switch_success", model = display)).await;
    Ok(())
}

pub(crate) async fn handle_model_command(
    bot: &Bot,
    msg: &Message,
    args: &str,
    config: &CliConfig,
    sessions: &SessionManager,
    cli: &AntigravityCli,
) -> Result<(), teloxide::RequestError> {
    if args.is_empty() {
        handle_model_command_empty(bot, msg, config, sessions, cli).await
    } else {
        handle_model_command_switch(bot, msg, args, config, sessions, cli).await
    }
}

pub(crate) fn eff_prov_model(s: &SessionData, cfg: &CliConfig) -> (String, String) {
    let p = SessionData::effective_provider(&s.provider, &cfg.provider).to_string();
    let m = SessionData::canonicalize_model_for_provider(&p, &s.model)
        .unwrap_or_else(|| SessionData::default_model_for_provider(&p, cfg.model.as_deref()));
    (p, m)
}

async fn apply_explicit_effort_switch(
    bot: &Bot,
    msg: &Message,
    raw: &str,
    key: &SessionKey,
    sess: &mut SessionData,
    ctx: (&CliConfig, &SessionManager, &AntigravityCli),
) -> Result<(), teloxide::RequestError> {
    let (config, sessions, cli) = ctx;
    if !is_valid_effort_level(raw) {
        let _ = send_reply(bot, msg, "❌ Invalid effort level. Supported levels: `minimal`, `low`, `medium`, `high`, `xhigh`, `max`.").await;
        return Ok(());
    }
    let mut applied_level = None;
    if let Ok(Some(updated)) = sessions.mutate_session(key, Some(sess), |cur| {
        let (cp, cm) = eff_prov_model(cur, config);
        if !session_supports_effort(&cp, &cm) { return false; }
        let lvl = clamp_session_effort(&cp, &cm, raw).unwrap_or_else(|| raw.to_string());
        cur.effort = Some(lvl.clone());
        applied_level = Some(lvl);
        true
    }).await {
        *sess = updated;
    }
    let Some(level) = applied_level else {
        let (_, cm) = eff_prov_model(sess, config);
        let _ = send_reply(bot, msg, format!("ℹ️ Model `{cm}` uses fixed reasoning and does not support `/effort` levels.")).await;
        return Ok(());
    };
    apply_model_to_active_pty(cli, sessions, sess).await;
    let _ = send_reply(bot, msg, format!("🤖 [tuner] Session reasoning effort switched to `{level}`.")).await;
    Ok(())
}

pub(crate) async fn handle_effort_command(
    bot: &Bot,
    msg: &Message,
    args: &str,
    config: &CliConfig,
    sessions: &SessionManager,
    cli: &AntigravityCli,
) -> Result<(), teloxide::RequestError> {
    let key = SessionKey::telegram(msg.chat.id.0, crate::telegram::get_topic_id(msg));
    let raw_prov = sessions.get_active(&key).await.ok().flatten().map(|s| s.provider).unwrap_or_default();
    if !matches!(SessionData::effective_provider(&raw_prov, &config.provider), "claude" | "codex") {
        crate::cli::antigravity::model_spec::ensure_live_catalog_fresh().await;
    }
    let dm = config.model.as_deref().unwrap_or("");
    let Ok((mut sess, _)) = sessions.get_or_init_session(&key, &config.provider, dm).await else { return Ok(()); };
    let (sp, eff_model, eff_fb) = SessionData::resolve_effective_target(Some(&sess), config);
    if !session_supports_effort(&sp, &eff_model) {
        if sess.effort.is_some() {
            let _ = sessions.mutate_session(&key, Some(&sess), |cur| {
                let (cp, cm) = eff_prov_model(cur, config);
                if !session_supports_effort(&cp, &cm) && cur.effort.is_some() {
                    cur.effort = None;
                    return true;
                }
                false
            }).await;
        }
        let _ = send_reply(bot, msg, format!("ℹ️ Model `{eff_model}` uses fixed reasoning and does not support `/effort` levels.")).await;
        return Ok(());
    }
    if args.is_empty() {
        super::lang::send_effort_menu(bot, msg, &sess, eff_fb.as_deref(), &eff_model).await;
        return Ok(());
    }
    apply_explicit_effort_switch(bot, msg, &args.trim().to_ascii_lowercase(), &key, &mut sess, (config, sessions, cli)).await
}
