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

pub(crate) fn resolve_target_topic(
    msg: &Message,
    args: &str,
    topic_cache: &super::TopicNameCache,
    default_tid: Option<i64>,
) -> Result<Option<i64>, String> {
    let trimmed = args.trim();
    let raw_cmd = trimmed.split_whitespace().next().unwrap_or("");
    let rest = trimmed[raw_cmd.len()..].trim();
    if rest.is_empty() {
        return Ok(default_tid);
    }
    if let Some(resolved_tid) = topic_cache.find_by_name(msg.chat.id.0, rest) {
        Ok(Some(resolved_tid))
    } else {
        Err(rest.to_string())
    }
}

async fn prepare_topic_reset(
    config: &CliConfig,
    sessions: &crate::session::manager::SessionManager,
    cli: &AntigravityCli,
    chat_id: i64,
    topic_id: Option<i64>,
) -> (String, String) {
    let key = crate::session::key::SessionKey::telegram(chat_id, topic_id);
    let default_model = config.model.as_deref().unwrap_or("antigravity-default");
    let mut preserved_provider = config.provider.clone();
    let mut preserved_model = default_model.to_string();

    if let Ok(Some(existing_sess)) = sessions.get_active(&key).await {
        if !existing_sess.provider.is_empty() {
            preserved_provider = existing_sess.provider.clone();
        }
        if !existing_sess.model.is_empty() && existing_sess.model != "antigravity-default" {
            preserved_model = existing_sess.model.clone();
        }
        let old_sid = existing_sess.get_session_id(&preserved_provider);
        if !old_sid.is_empty() {
            cli.sessions.terminate(&old_sid).await;
            let _ = crate::cli::claude::terminate_topic(chat_id, topic_id, Some(&old_sid)).await;
            let _ = crate::cli::codex::terminate_topic(chat_id, topic_id, Some(&old_sid)).await;
        }
    }
    let _ = cli.sessions.abort(chat_id, topic_id).await;
    let _ = crate::cli::claude::terminate_topic(chat_id, topic_id, None).await;
    let _ = crate::cli::codex::terminate_topic(chat_id, topic_id, None).await;
    (preserved_provider, preserved_model)
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
    let target_topic = match resolve_target_topic(msg, args, topic_cache, topic_id) {
        Ok(tid) => tid,
        Err(name) => {
            let _ = send_reply(bot, msg, t!("bot.unknown_topic", name = name)).await;
            return Ok(true);
        }
    };

    let (preserved_provider, preserved_model) = prepare_topic_reset(
        config, sessions, cli, msg.chat.id.0, target_topic,
    ).await;

    let key = crate::session::key::SessionKey::telegram(msg.chat.id.0, target_topic);
    let Ok(mut sess) = sessions.reset_provider_session(&key, &preserved_provider, &preserved_model).await else {
        return Ok(true);
    };

    if sess.topic_name.is_none()
        && let Some(tid) = target_topic
        && let Some(name) = topic_cache.find_by_id(msg.chat.id.0, tid)
    {
        sess.topic_name = Some(name);
        let _ = sessions.preserve_session_identity(&sess).await;
    }

    let sid = crate::telegram::session_init::initialize_session_with_prompt(
        bot, msg, sessions, &mut sess, cli, config, None,
    ).await?;
    if !sid.is_empty() {
        let mut target_msg = msg.clone();
        if let Some(tid) = sess.topic_id {
            target_msg.thread_id = Some(teloxide::types::ThreadId(teloxide::types::MessageId(tid as i32)));
        }
        crate::telegram::async_observer::spawn_session_async_observer(
            bot.clone(), &target_msg, sid, cli.clone(), sessions.clone(), config.clone(),
        );
    }
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
        let interrupted = agy_int || claude_int || codex_int;
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
        crate::cli::claude::terminate_all_for_chat(msg.chat.id.0).await;
        crate::cli::codex::terminate_all_for_chat(msg.chat.id.0).await;
        let _ = send_reply(bot, msg, t!("bot.stop_all_success")).await;
        return Ok(true);
    }
    Ok(false)
}

async fn try_query_live_memory(
    bot: &Bot,
    msg: &Message,
    config: &CliConfig,
    sessions: &crate::session::manager::SessionManager,
    cli: &AntigravityCli,
) -> bool {
    let key = crate::session::key::SessionKey::telegram(msg.chat.id.0, crate::telegram::get_topic_id(msg));
    let model = config.model.as_deref().unwrap_or("antigravity-default");
    if let Ok((mut sess, _)) = sessions.resolve_session(&key, &config.provider, model).await
        && let Ok(sid) = super::session_init::initialize_session_if_needed(bot, msg, sessions, &mut sess, cli, config).await
        && !sid.is_empty() && !sid.starts_with("mock-")
    {
        let prompt = "현재 세션의 시스템 프롬프트 `<user_rules>`(Rules Context)에 주입되어 있는 `mainmemory.md` 내용(About the User, Core System Architecture & Roles, Decisions & Preferences)과 적용된 룰 상태를 요약 없이 있는 그대로 출력해줘.";
        let _ = super::run_cli_stream(bot, msg, prompt, &sid, cli, (sessions, sess), config).await;
        return true;
    }
    false
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
    sessions: &crate::session::manager::SessionManager,
    cli: &AntigravityCli,
) -> Result<(), teloxide::RequestError> {
    if try_query_live_memory(bot, msg, config, sessions, cli).await {
        return Ok(());
    }
    send_static_memory_fallback(bot, msg, config).await
}
