//! # Telegram CLI Provider Command & Callback Handler
//!
//! ## Overview
//! Handles the `/provider` slash command, inline provider selection keyboard, and
//! `provider:<name>` callback queries across `antigravity`, `claude`, and `codex`.
//!
//! ## Search Tags
//! #provider-command, #provider-callback, #multi-provider, #slash-sync

use teloxide::prelude::*;
use teloxide::types::MessageId;
use crate::config::CliConfig;
use crate::session::{data::SessionData, key::SessionKey, manager::SessionManager};
use crate::cli::{AskQuestionData, antigravity::{AntigravityCli, session::AskState}};

pub(crate) async fn is_stale_ask_session(
    sessions: &SessionManager,
    config: &CliConfig,
    session_data: &SessionData,
    prov: &str,
    sess_id: &str,
) -> bool {
    let want_p = SessionData::effective_provider(prov, &config.provider);
    match sessions.get_active(&session_data.session_key()).await {
        Ok(Some(latest)) => {
            let lp = SessionData::effective_provider(&latest.provider, &config.provider);
            lp != want_p || (!sess_id.is_empty() && latest.get_session_id(want_p) != sess_id)
        }
        Ok(None) => !sess_id.is_empty(),
        Err(_) => false,
    }
}

pub(crate) async fn register_and_verify_ask_state(
    bot: &Bot,
    chat_id: ChatId,
    msg_id: i32,
    ask: Vec<AskQuestionData>,
    session_data: &SessionData,
    ctx: (&CliConfig, &AntigravityCli, &SessionManager),
) {
    let (config, cli, sessions) = ctx;
    let prov = SessionData::effective_provider(&session_data.provider, &config.provider);
    let sess_id = session_data.get_session_id(prov);
    let bmp = if ask[0].is_multi_select { "0".repeat(ask[0].options.len()) } else { String::new() };
    let n = ask.len();
    cli.sessions.set_ask_state(&sess_id, AskState {
        msg_id, questions: ask, current_index: 0, answers: vec![String::new(); n], current_bitmap: bmp, waiting_for_write_in: false,
    }).await;
    if is_stale_ask_session(sessions, config, session_data, prov, &sess_id).await {
        cancel_abandoned_provider_ask(cli, prov, chat_id.0, session_data.topic_id, &sess_id).await;
        if bot.delete_message(chat_id, MessageId(msg_id)).await.is_err() {
            let _ = bot.edit_message_reply_markup(chat_id, MessageId(msg_id)).await;
        }
    }
}

pub(crate) async fn apply_model_to_active_pty(
    cli: &AntigravityCli,
    sessions: &SessionManager,
    sess: &SessionData,
) {
    let sp = SessionData::effective_provider(&sess.provider, &cli.config.provider);
    let sid = sess.get_session_id(sp);
    if sp == "codex" || (sp != "claude" && sid.is_empty()) {
        return;
    }
    let lk = if sid.is_empty() { format!("claude:{}:{:?}", sess.chat_id, sess.topic_id) } else { sid.clone() };
    let lock = cli.pty_sync.lock_for(&lk).await;
    let _guard = lock.lock().await;
    let key = SessionKey::telegram(sess.chat_id, sess.topic_id);
    let latest = sessions.get_active(&key).await.ok().flatten().unwrap_or_else(|| sess.clone());
    let lp = SessionData::effective_provider(&latest.provider, &cli.config.provider);
    if lp != sp || latest.get_session_id(lp) != sid {
        return;
    }
    if lp == "claude" {
        if !cli.is_pty_busy_or_asking(&sid, latest.chat_id, latest.topic_id).await {
            let _ = crate::cli::claude::set_model_live(latest.chat_id, latest.topic_id, (!sid.is_empty()).then_some(sid.as_str()), &latest.model).await;
        }
        return;
    }
    if lp == "codex" {
        return;
    }
    crate::cli::antigravity::model_spec::ensure_live_catalog_fresh().await;
    let desired = crate::cli::antigravity::model_spec::format_pty_model_target(&latest.model, latest.effort.as_deref(), cli.config.effort.as_deref());
    cli.sync_locked_idle_pty(&sid, latest.chat_id, latest.topic_id, desired).await;
}

fn build_provider_keyboard() -> teloxide::types::InlineKeyboardMarkup {
    use teloxide::types::InlineKeyboardButton as Btn;
    teloxide::types::InlineKeyboardMarkup::new(vec![
        vec![Btn::callback("🚀 Antigravity (agy)", "provider:antigravity")],
        vec![Btn::callback("🟠 Claude Code (claude)", "provider:claude")],
        vec![Btn::callback("🟢 OpenAI Codex (codex)", "provider:codex")],
    ])
}

pub(crate) async fn cancel_abandoned_provider_ask(
    cli: &AntigravityCli,
    prov: &str,
    chat_id: i64,
    topic_id: Option<i64>,
    sid: &str,
) {
    match SessionData::canonical_provider(prov).unwrap_or("antigravity") {
        "claude" => {
            let _ = crate::cli::claude::interrupt_session(chat_id, topic_id).await;
        }
        "codex" => {
            let _ = crate::cli::codex::interrupt_session(chat_id, topic_id).await;
        }
        _ if !sid.is_empty() => {
            let lock = cli.pty_sync.lock_for(sid).await;
            let _g = lock.lock().await;
            if cli.sessions.is_active(sid).await {
                for _ in 0..16 {
                    if !cli.sessions.is_running(sid).await { break; }
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                }
                if !cli.sessions.is_running(sid).await && !cli.sessions.is_async_running(sid).await {
                    let _ = cli.sessions.write_to_session(sid, "\x1b").await;
                }
            }
        }
        _ => {}
    }
    if !sid.is_empty() {
        cli.sessions.set_ask_active(sid, false).await;
    }
}

pub(crate) async fn switch_session_provider(
    target: &str,
    config: &CliConfig,
    sessions: &SessionManager,
    cli: &AntigravityCli,
    sess: &mut crate::session::data::SessionData,
) -> String {
    let target = SessionData::canonical_provider(target).unwrap_or("antigravity");
    if target == "antigravity" {
        crate::cli::antigravity::model_spec::ensure_live_catalog_fresh().await;
    }
    let mut old_prov = SessionData::effective_provider(&sess.provider, &config.provider).to_string();
    let mut old_sid = sess.get_session_id(&old_prov);
    let mut status_text = String::new();
    if let Ok(Some(updated)) = sessions
        .mutate_session(&sess.session_key(), Some(sess), |cur| {
            old_prov = SessionData::effective_provider(&cur.provider, &config.provider).to_string();
            old_sid = cur.get_session_id(&old_prov);
            status_text = cur.apply_provider_switch(target);
            true
        })
        .await
    {
        *sess = updated;
    } else {
        status_text = sess.apply_provider_switch(target);
    }
    if old_prov != target {
        let has_open_ask = (!old_sid.is_empty() && cli.sessions.is_ask_active(&old_sid).await)
            || (old_prov == "claude" && crate::cli::claude::has_pending_ask(sess.chat_id, sess.topic_id, None).await)
            || (old_prov == "codex" && crate::cli::codex::has_pending_ask(sess.chat_id, sess.topic_id, None).await);
        if has_open_ask {
            cancel_abandoned_provider_ask(cli, &old_prov, sess.chat_id, sess.topic_id, &old_sid).await;
        }
    }
    super::commands_model::apply_model_to_active_pty(cli, sessions, sess).await;
    status_text
}

async fn execute_provider_switch(
    bot: &Bot,
    chat_id: ChatId,
    target: &str,
    config: &CliConfig,
    sessions: &SessionManager,
    cli: &AntigravityCli,
    sess: &mut crate::session::data::SessionData,
) -> String {
    let status_text = switch_session_provider(target, config, sessions, cli, sess).await;
    super::commands_registry::update_chat_commands_for_provider(bot, chat_id, target).await;
    status_text
}

/// Handles the `/provider` slash command and optional `<name>` argument.
pub async fn handle_provider_command(
    bot: &Bot,
    msg: &Message,
    args: &str,
    config: &CliConfig,
    sessions: &SessionManager,
    cli: &AntigravityCli,
) -> Result<(), teloxide::RequestError> {
    let key = crate::session::key::SessionKey::telegram(msg.chat.id.0, crate::telegram::get_topic_id(msg));
    let dm = config.model.as_deref().unwrap_or("");
    let Ok((mut sess, _)) = sessions.get_or_init_session(&key, &config.provider, dm).await else {
        return Ok(());
    };
    if args.is_empty() {
        let text = super::formatting::markdown_to_telegram_html(&format!(
            "Current CLI provider is `{}` (model: `{}`). Select a CLI provider:",
            sess.provider, sess.model
        ));
        let mut req = bot.send_message(msg.chat.id, text).parse_mode(teloxide::types::ParseMode::Html);
        if let Some(tid) = msg.thread_id { req = req.message_thread_id(tid); }
        let _ = req.reply_markup(build_provider_keyboard()).await;
    } else if let Some(target) = crate::session::data::SessionData::canonical_provider(args) {
        let reply_msg = execute_provider_switch(bot, msg.chat.id, target, config, sessions, cli, &mut sess).await;
        let _ = super::commands::send_reply(bot, msg, reply_msg).await;
    } else {
        let _ = super::commands::send_reply(
            bot, msg, "❌ Invalid provider. Supported providers: `antigravity` (`agy`), `claude`, `codex`.",
        ).await;
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
    cli: &AntigravityCli,
) {
    let Some(target) = crate::session::data::SessionData::canonical_provider(raw_provider) else {
        return;
    };
    let key = crate::session::key::SessionKey::telegram(msg.chat.id.0, crate::telegram::get_topic_id(msg));
    let dm = config.model.as_deref().unwrap_or("");
    if let Ok((mut s, _)) = sessions.get_or_init_session(&key, &config.provider, dm).await {
        let status_text = execute_provider_switch(bot, msg.chat.id, target, config, sessions, cli, &mut s).await;
        super::callbacks_effort::edit_markdown_status(bot, msg, &status_text).await;
    }
}
