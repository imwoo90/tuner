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

pub(crate) fn parse_new_command_args<'a>(
    msg: &Message,
    args: &'a str,
    topic_cache: &super::TopicNameCache,
    default_tid: Option<i64>,
) -> (Option<i64>, Option<&'a str>) {
    let trimmed = args.trim();
    let raw_cmd = trimmed.split_whitespace().next().unwrap_or("");
    let rest = trimmed[raw_cmd.len()..].trim();
    if rest.is_empty() {
        return (default_tid, None);
    }
    if let Some(resolved_tid) = topic_cache.find_by_name(msg.chat.id.0, rest) {
        return (Some(resolved_tid), None);
    }
    (default_tid, Some(rest))
}

async fn prepare_topic_reset(
    config: &CliConfig,
    sessions: &crate::session::manager::SessionManager,
    cli: &AntigravityCli,
    chat_id: i64,
    topic_id: Option<i64>,
) -> String {
    let key = crate::session::key::SessionKey::telegram(chat_id, topic_id);
    let default_model = config.model.as_deref().unwrap_or("antigravity-default");
    let mut preserved_model = default_model.to_string();

    if let Ok(Some(existing_sess)) = sessions.get_active(&key).await {
        if !existing_sess.model.is_empty() && existing_sess.model != "antigravity-default" {
            preserved_model = existing_sess.model.clone();
        }
        let old_sid = existing_sess.get_session_id(&config.provider);
        if !old_sid.is_empty() {
            cli.sessions.terminate(&old_sid).await;
        }
    }
    preserved_model
}

pub(crate) async fn handle_new_command(
    bot: &Bot,
    msg: &Message,
    args: &str,
    config: &CliConfig,
    sessions: &crate::session::manager::SessionManager,
    cli: &AntigravityCli,
    topic_cache: &super::TopicNameCache,
) -> Result<bool, teloxide::RequestError> {
    let topic_id = crate::telegram::get_topic_id(msg);
    let (target_topic, inline_prompt) = parse_new_command_args(msg, args, topic_cache, topic_id);

    let preserved_model = prepare_topic_reset(
        config, sessions, cli, msg.chat.id.0, target_topic,
    ).await;

    let key = crate::session::key::SessionKey::telegram(msg.chat.id.0, target_topic);
    let Ok(mut sess) = sessions.reset_provider_session(&key, &config.provider, &preserved_model).await else {
        return Ok(true);
    };

    if sess.topic_name.is_none()
        && let Some(tid) = target_topic
        && let Some(name) = topic_cache.find_by_id(msg.chat.id.0, tid)
    {
        sess.topic_name = Some(name);
        let _ = sessions.preserve_session_identity(&sess).await;
    }

    if inline_prompt.is_some() {
        return Ok(false);
    }

    let _ = crate::telegram::session_init::initialize_session_with_prompt(
        bot, msg, sessions, &mut sess, cli, config, None,
    ).await;
    Ok(true)
}

pub(crate) async fn handle_session_control_commands(
    bot: &Bot,
    msg: &Message,
    text: &str,
    config: &CliConfig,
    sessions: &crate::session::manager::SessionManager,
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
        let interrupted = cli.sessions.interrupt(msg.chat.id.0, topic_id).await;
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
        let _ = send_reply(bot, msg, t!("bot.stop_all_success")).await;
        return Ok(true);
    }
    Ok(false)
}
