//! # Session Control Slash Commands
//!
//! ## Overview
//! Handles session lifecycle slash commands: `/new`, `/reset`, `/stop`, `/stop_all`, and `/abort`.
//!
//! ## Search Tags
//! #session-control, #stop-command, #abort-command, #new-session

use teloxide::prelude::*;
use crate::config::CliConfig;
use crate::cli::antigravity::AntigravityCli;
use crate::t;
use super::commands::send_reply;

pub(crate) use super::handler::{is_topic_busy_or_asking, resolve_target_topic};

async fn clear_topic_asks(
    sessions: &crate::session::manager::SessionManager,
    cli: &AntigravityCli,
    chat_id: i64,
    topic_id: Option<i64>,
) -> Vec<String> {
    let key = crate::session::key::SessionKey::telegram(chat_id, topic_id);
    let mut cleared = Vec::new();
    if let Ok(Some(sess)) = sessions.get_active(&key).await {
        for ps in sess.provider_sessions.values() {
            if !ps.session_id.is_empty() && cli.sessions.is_ask_active(&ps.session_id).await {
                cli.sessions.set_ask_active(&ps.session_id, false).await;
                cleared.push(ps.session_id.clone());
            }
        }
    }
    cleared
}

async fn prepare_topic_reset(
    config: &CliConfig,
    sessions: &crate::session::manager::SessionManager,
    cli: &AntigravityCli,
    chat_id: i64,
    topic_id: Option<i64>,
) -> Option<String> {
    let key = crate::session::key::SessionKey::telegram(chat_id, topic_id);
    let mut initial_prov = None;
    let mut sids_to_clean = Vec::new();
    if let Ok(Some(existing_sess)) = sessions.get_active(&key).await {
        let ap = crate::session::data::SessionData::effective_provider(&existing_sess.provider, &config.provider).to_string();
        let old_sid = existing_sess.get_session_id(&ap);
        if !old_sid.is_empty() {
            sids_to_clean.push(old_sid);
        }
        initial_prov = Some(ap);
    }
    let holder_sids: Vec<String> = cli
        .sessions
        .holders
        .lock()
        .await
        .iter()
        .filter(|(_, h)| h.chat_id == Some(chat_id) && h.topic_id == topic_id)
        .map(|(sid, _)| sid.clone())
        .collect();
    for sid in holder_sids {
        if !sids_to_clean.contains(&sid) {
            sids_to_clean.push(sid);
        }
    }
    for old_sid in &sids_to_clean {
        cli.terminate_session_clean(old_sid).await;
        let _ = crate::cli::claude::terminate_topic(chat_id, topic_id, Some(old_sid)).await;
        let _ = crate::cli::codex::terminate_topic(chat_id, topic_id, Some(old_sid)).await;
    }
    let _ = cli.sessions.abort(chat_id, topic_id).await;
    let _ = crate::cli::claude::terminate_topic(chat_id, topic_id, None).await;
    let _ = crate::cli::codex::terminate_topic(chat_id, topic_id, None).await;
    initial_prov
}

pub(crate) async fn handle_new_command(
    bot: &Bot,
    msg: &Message,
    args: &str,
    config: &CliConfig,
    sessions: &std::sync::Arc<crate::session::manager::SessionManager>,
    cli: &AntigravityCli,
    topic_cache: &super::TopicNameCache,
) -> Result<bool, teloxide::RequestError> {
    let topic_id = crate::telegram::get_topic_id(msg);
    let target_res = super::handler::LOCKED_TOPIC_ID
        .try_with(|r| r.clone())
        .unwrap_or_else(|_| resolve_target_topic(msg, args, topic_cache, topic_id));
    let target_topic = match target_res {
        Ok(tid) => tid,
        Err(name) => {
            let _ = send_reply(bot, msg, t!("bot.unknown_topic", name = name)).await;
            return Ok(true);
        }
    };

    let init_prov = prepare_topic_reset(config, sessions, cli, msg.chat.id.0, target_topic).await;

    let key = crate::session::key::SessionKey::telegram(msg.chat.id.0, target_topic);
    let default_model = config.model.as_deref().unwrap_or("");
    let topic_hint = target_topic.and_then(|tid| topic_cache.find_by_id(msg.chat.id.0, tid));
    let Ok(mut sess) = sessions.reset_topic_sessions(&key, &config.provider, default_model, init_prov.as_deref(), topic_hint).await else {
        return Ok(true);
    };

    let mut target_msg = msg.clone();
    target_msg.thread_id = target_topic.map(|tid| teloxide::types::ThreadId(teloxide::types::MessageId(tid as i32)));
    target_msg.is_topic_message = target_topic.is_some();
    let _ = crate::telegram::session_init::ensure_session_ready_for_turn(
        bot, &target_msg, sessions, &mut sess, cli, config,
    ).await?;
    crate::messenger::web::events_bridge::emit_s(&sess).await;
    Ok(true)
}

pub(crate) async fn handle_session_control_commands(
    bot: &Bot,
    msg: &Message,
    text: &str,
    config: &CliConfig,
    sessions: &std::sync::Arc<crate::session::manager::SessionManager>,
    cli: &AntigravityCli,
    topic_cache: &super::TopicNameCache,
) -> Result<bool, teloxide::RequestError> {
    let args = text.trim();
    let raw_cmd = args.split_whitespace().next().unwrap_or("");
    let cmd = raw_cmd.split('@').next().unwrap_or(raw_cmd);
    let topic_id = crate::telegram::get_topic_id(msg);

    if cmd == "/new" || cmd == "/reset" {
        return handle_new_command(bot, msg, args, config, sessions, cli, topic_cache).await;
    }
    if cmd == "/stop" {
        let agy_int = cli.sessions.interrupt(msg.chat.id.0, topic_id).await;
        let claude_int = crate::cli::claude::interrupt_session(msg.chat.id.0, topic_id).await;
        let codex_int = crate::cli::codex::interrupt_session(msg.chat.id.0, topic_id).await;
        let cleared_asks = clear_topic_asks(sessions, cli, msg.chat.id.0, topic_id).await;
        crate::messenger::web::events_bridge::tg_stop(msg.chat.id.0, topic_id, &cleared_asks).await;
        let interrupted = agy_int || claude_int || codex_int || !cleared_asks.is_empty();
        let reply_text = if interrupted {
            t!("bot.stop_interrupted")
        } else {
            t!("bot.stop_none")
        };
        let _ = send_reply(bot, msg, reply_text).await;
        return Ok(true);
    }
    if cmd == "/stop_all" || cmd == "/abort" {
        cli.sessions.terminate_all().await;
        crate::cli::claude::terminate_all().await;
        crate::cli::codex::terminate_all().await;
        let cleared_asks: Vec<String> = cli.sessions.active_asks.lock().await.drain().map(|(k, _)| k).collect();
        cli.clear_all_pty_applied().await;
        crate::messenger::web::events_bridge::tg_stop_all(&cleared_asks).await;
        let _ = send_reply(bot, msg, t!("bot.stop_all_success")).await;
        return Ok(true);
    }
    Ok(false)
}

pub(crate) async fn try_query_live_memory(
    bot: &Bot,
    msg: &Message,
    config: &CliConfig,
    sessions: &std::sync::Arc<crate::session::manager::SessionManager>,
    cli: &AntigravityCli,
) -> bool {
    let topic_id = crate::telegram::get_topic_id(msg);
    let lock = sessions.lock_pool.get((msg.chat.id.0, topic_id));
    let Ok(_topic_guard) = lock.try_lock() else {
        return false;
    };
    if is_topic_busy_or_asking(sessions, cli, msg.chat.id.0, topic_id).await {
        return false;
    }
    let key = crate::session::key::SessionKey::telegram(msg.chat.id.0, topic_id);
    let model = config.model.as_deref().unwrap_or("");
    let Ok((mut sess, _)) = sessions.resolve_session(&key, &config.provider, model).await else {
        return false;
    };
    let Ok(mut sid) = super::session_init::ensure_session_ready_for_turn(
        bot, msg, sessions, &mut sess, cli, config,
    ).await else {
        return false;
    };
    if let Ok(Some(latest)) = sessions.get_active(&key).await {
        sess = latest;
        let cp = crate::session::data::SessionData::effective_provider(&sess.provider, &config.provider);
        if sess.get_session_id(cp) != sid {
            let Ok(re_sid) = super::session_init::ensure_session_ready_for_turn(
                bot, msg, sessions, &mut sess, cli, config,
            ).await else {
                return false;
            };
            sid = re_sid;
        }
    }
    if sid.is_empty()
        || sid.starts_with("mock-")
        || is_topic_busy_or_asking(sessions, cli, msg.chat.id.0, topic_id).await
    {
        return false;
    }
    let prompt = "현재 세션의 시스템 프롬프트 `<user_rules>`(Rules Context)에 주입되어 있는 `mainmemory.md` 내용(About the User, Core System Architecture & Roles, Decisions & Preferences)과 적용된 룰 상태를 요약 없이 있는 그대로 출력해줘.";
    let _ = super::run_cli_stream(bot, msg, prompt, &sid, cli, (sessions, sess), config).await;
    true
}

async fn send_static_memory_fallback(bot: &Bot, msg: &Message, config: &CliConfig) -> Result<(), teloxide::RequestError> {
    let p1 = config.working_dir.join(".agents/rules/mainmemory.md");
    let path = if p1.is_file() { p1 } else { config.working_dir.join("memory_system/MAINMEMORY.md") };
    let raw = std::fs::read_to_string(path).unwrap_or_else(|_| t!("bot.memory_empty"));
    let content = raw.strip_prefix("---").and_then(|s| s.find("---").map(|i| s[i + 3..].trim_start())).unwrap_or(&raw);
    let html = crate::telegram::formatting::markdown_to_telegram_html(content);
    for chunk in crate::telegram::formatting::split_html_message(&html, 4000) {
        let mut req = bot.send_message(msg.chat.id, chunk).parse_mode(teloxide::types::ParseMode::Html);
        if let Some(tid) = msg.thread_id { req = req.message_thread_id(tid); }
        let _ = req.await;
    }
    Ok(())
}

pub(crate) async fn handle_memory_command(
    bot: &Bot,
    msg: &Message,
    config: &CliConfig,
    sessions: &std::sync::Arc<crate::session::manager::SessionManager>,
    cli: &AntigravityCli,
) -> Result<(), teloxide::RequestError> {
    if try_query_live_memory(bot, msg, config, sessions, cli).await {
        return Ok(());
    }
    send_static_memory_fallback(bot, msg, config).await
}
