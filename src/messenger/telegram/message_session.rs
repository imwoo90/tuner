//! # Turn-Gated Message Session & Directive Resolver
//!
//! ## Overview
//! Resolves active or overdue `SessionData` strictly when a real user turn executes,
//! keeping `@model` preference changes, attachment captions, and rejected directives side-effect-free.
//!
//! ## Search Tags
//! #message-session, #directive-resolution, #session-freshness, #ask-guard

use teloxide::prelude::*;
use crate::config::CliConfig;
use crate::session::manager::SessionManager;

pub(crate) async fn get_live_open_ask_session(
    msg: &Message,
    config: &CliConfig,
    sessions: &SessionManager,
    cli: &crate::cli::antigravity::AntigravityCli,
) -> Option<crate::session::data::SessionData> {
    let tid = super::get_topic_id(msg);
    let key = crate::session::key::SessionKey::telegram(msg.chat.id.0, tid);
    let sess = sessions.get_active(&key).await.ok().flatten()?;
    let prov = crate::session::data::SessionData::effective_provider(&sess.provider, &config.provider);
    let sid = sess.get_session_id(prov);
    if sid.is_empty() || !cli.sessions.is_ask_active(&sid).await {
        return None;
    }
    if cli.is_ask_backing_alive(prov, msg.chat.id.0, tid, &sid).await {
        Some(sess)
    } else {
        cli.sessions.set_ask_active(&sid, false).await;
        None
    }
}

async fn load_turn_session(
    msg: &Message,
    config: &CliConfig,
    sessions: &SessionManager,
    cli: &crate::cli::antigravity::AntigravityCli,
) -> Option<crate::session::data::SessionData> {
    let tid = super::get_topic_id(msg);
    let key = crate::session::key::SessionKey::telegram(msg.chat.id.0, tid);
    let dm = config.model.as_deref().unwrap_or("");
    let res = if super::handler::is_topic_busy_or_asking(sessions, cli, msg.chat.id.0, tid).await {
        sessions.get_or_init_session(&key, &config.provider, dm).await
    } else {
        sessions.resolve_session(&key, &config.provider, dm).await
    };
    res.ok().map(|(s, _)| s)
}

async fn peek_directive_context(
    msg: &Message,
    text: &str,
    config: &CliConfig,
    sessions: &SessionManager,
) -> (String, Option<String>) {
    use crate::session::data::SessionData;
    let key = crate::session::key::SessionKey::telegram(msg.chat.id.0, super::get_topic_id(msg));
    let mut existing = sessions.get_active(&key).await.ok().flatten();
    let mut ep = SessionData::effective_provider(
        existing.as_ref().map(|s| s.provider.as_str()).unwrap_or(""),
        &config.provider,
    )
    .to_string();
    if text.trim_start().starts_with('@') || !matches!(ep.as_str(), "claude" | "codex") {
        if !matches!(ep.as_str(), "claude" | "codex") {
            crate::cli::antigravity::model_spec::ensure_live_catalog_fresh().await;
        }
        existing = sessions.get_active(&key).await.ok().flatten();
        ep = SessionData::effective_provider(
            existing.as_ref().map(|s| s.provider.as_str()).unwrap_or(""),
            &config.provider,
        )
        .to_string();
    }
    let eff = existing
        .and_then(|s| s.effort.or_else(|| SessionData::extract_compatible_effort_suffix(&ep, &s.model)))
        .or_else(|| SessionData::configured_effort_fallback(config, &ep));
    (ep, eff)
}

async fn persist_directive_model(
    msg: &Message,
    model_eff: (&str, Option<&str>),
    config: &CliConfig,
    sessions: &SessionManager,
) -> Result<Option<crate::session::data::SessionData>, String> {
    use crate::session::data::SessionData;
    let (canonical_mo, resolved_eff) = model_eff;
    let key = crate::session::key::SessionKey::telegram(msg.chat.id.0, super::get_topic_id(msg));
    let dm = config.model.as_deref().unwrap_or("");
    let Ok((mut sess, _)) = sessions.get_or_init_session(&key, &config.provider, dm).await else {
        return Ok(None);
    };
    let mut mismatch_prov = None;
    if let Ok(Some(updated)) = sessions
        .mutate_session(&key, Some(&sess), |cur| {
            let cp = SessionData::effective_provider(&cur.provider, &config.provider);
            if !SessionData::is_model_compatible(canonical_mo, cp) {
                mismatch_prov = Some(cp.to_string());
                return false;
            }
            cur.provider = cp.to_string();
            cur.model = canonical_mo.to_string();
            cur.effort = resolved_eff.map(str::to_string);
            true
        })
        .await
    {
        sess = updated;
    } else if mismatch_prov.is_none() {
        sess.model = canonical_mo.to_string();
        sess.effort = resolved_eff.map(str::to_string);
    }
    match mismatch_prov {
        Some(cp) => Err(cp),
        None => Ok(Some(sess)),
    }
}

async fn apply_resolved_directive<'a>(
    bot: &Bot,
    msg: &Message,
    resolved: (String, Option<String>, &'a str),
    config: &CliConfig,
    sessions: &SessionManager,
    cli: &crate::cli::antigravity::AntigravityCli,
) -> Result<Option<(crate::session::data::SessionData, &'a str)>, teloxide::RequestError> {
    let (canonical_mo, resolved_eff, rem) = resolved;
    let sess = match persist_directive_model(
        msg,
        (&canonical_mo, resolved_eff.as_deref()),
        config,
        sessions,
    )
    .await
    {
        Ok(Some(s)) => s,
        Ok(None) => return Ok(None),
        Err(cp) => {
            let _ = super::commands::send_reply(bot, msg, format!("⚠️ Model `{canonical_mo}` is not available for active provider `{cp}`.")).await;
            return Ok(None);
        }
    };
    if rem.is_empty() {
        super::commands_model::apply_model_to_active_pty(cli, sessions, &sess).await;
        let disp = match resolved_eff {
            Some(e) => format!("{canonical_mo} (effort: {e})"),
            None => canonical_mo,
        };
        let _ = super::commands::send_reply(bot, msg, format!("🤖 [tuner] Next message will use `{disp}`.")).await;
        return Ok(None);
    }
    let Some(turn_sess) = load_turn_session(msg, config, sessions, cli).await else {
        return Ok(None);
    };
    super::commands_model::apply_model_to_active_pty(cli, sessions, &turn_sess).await;
    Ok(Some((turn_sess, rem)))
}

pub(crate) async fn sync_live_peer_ask_session(
    sessions: &SessionManager,
    config: &CliConfig,
    session_data: &crate::session::data::SessionData,
    prov: &str,
    chat_id: i64,
) -> crate::session::data::SessionData {
    use crate::session::data::SessionData;
    let start_sid = session_data.get_session_id(prov);
    let live_sid = match prov {
        "claude" => crate::cli::claude::active_session_id(chat_id, session_data.topic_id).await,
        "codex" => crate::cli::codex::active_session_id(chat_id, session_data.topic_id).await,
        _ => None,
    };
    let Some(new_sid) = live_sid.filter(|s| !s.is_empty() && s != &start_sid) else {
        return session_data.clone();
    };
    let mut synced = session_data.clone();
    synced.set_session_id(prov, &new_sid);
    let fb = start_sid.is_empty().then_some(&synced);
    let updated = sessions
        .mutate_session(&session_data.session_key(), fb, |cur| {
            let lp = SessionData::effective_provider(&cur.provider, &config.provider);
            if lp != prov
                || cur.get_session_id(prov) != start_sid
                || (!start_sid.is_empty() && cur.get_past_session_ids(prov).contains(&start_sid))
            {
                return false;
            }
            cur.set_session_id(prov, &new_sid);
            true
        })
        .await;
    match updated {
        Ok(Some(s)) => s,
        _ => session_data.clone(),
    }
}

pub(crate) async fn has_leading_directive(
    msg: &Message,
    text: &str,
    config: &CliConfig,
    sessions: &SessionManager,
) -> bool {
    use super::reply_directives::{self, DirectiveOutcome};
    if !text.trim_start().starts_with('@') {
        return false;
    }
    let (ep, eff_fb) = peek_directive_context(msg, text, config, sessions).await;
    !matches!(
        reply_directives::resolve_directive_for_provider(&ep, eff_fb.as_deref(), text),
        DirectiveOutcome::None
    )
}

pub(crate) async fn resolve_message_session<'a>(
    bot: &Bot,
    msg: &Message,
    text: &'a str,
    config: &CliConfig,
    sessions: &SessionManager,
    cli: &crate::cli::antigravity::AntigravityCli,
) -> Result<Option<(crate::session::data::SessionData, &'a str)>, teloxide::RequestError> {
    use super::reply_directives::{self, DirectiveOutcome};
    let (ep, eff_fb) = peek_directive_context(msg, text, config, sessions).await;
    match reply_directives::resolve_directive_for_provider(&ep, eff_fb.as_deref(), text) {
        DirectiveOutcome::InvalidEffort => {
            let _ = super::commands::send_reply(bot, msg, "❌ Invalid effort level. Supported levels: `minimal`, `low`, `medium`, `high`, `xhigh`, `max`.").await;
            Ok(None)
        }
        DirectiveOutcome::IncompatibleModel(m) => {
            let _ = super::commands::send_reply(bot, msg, format!("⚠️ Model `{m}` is not available for active provider `{ep}`.")).await;
            Ok(None)
        }
        DirectiveOutcome::Resolved(cm, eo, rem) => {
            apply_resolved_directive(bot, msg, (cm, eo, rem), config, sessions, cli).await
        }
        DirectiveOutcome::None => {
            Ok(load_turn_session(msg, config, sessions, cli).await.map(|s| (s, text.trim())))
        }
    }
}
