//! # Session Control Slash Commands
//!
//! Handles session lifecycle slash commands: `/new`, `/reset`, `/stop`, `/stop_all`, and `/abort`.
//!
//! ## Search Tags
//! #session-control, #stop-command, #abort-command, #new-session

use teloxide::prelude::*;
use crate::config::CliConfig;
use crate::cli::antigravity::AntigravityCli;
use crate::t;
use super::commands::send_reply;

pub(crate) async fn handle_new_command(
    bot: &Bot,
    msg: &Message,
    args: &str,
    mut topic_id: Option<i64>,
    config: &CliConfig,
    sessions: &crate::session::manager::SessionManager,
    cli: &AntigravityCli,
    topic_cache: &super::TopicNameCache,
) -> Result<(), teloxide::RequestError> {
    let parts: Vec<&str> = args.split_whitespace().collect();
    if parts.len() > 1 {
        let name = parts[1];
        if let Some(resolved_tid) = topic_cache.find_by_name(msg.chat.id.0, name) {
            topic_id = Some(resolved_tid);
        } else {
            let _ = send_reply(bot, msg, t!("bot.unknown_topic", name = name)).await;
            return Ok(());
        }
    }
    let key = crate::session::key::SessionKey::telegram(msg.chat.id.0, topic_id);
    if let Ok(Some(existing_sess)) = sessions.get_active(&key).await {
        let old_sid = existing_sess.get_session_id(&config.provider);
        if !old_sid.is_empty() {
            cli.sessions.terminate(&old_sid).await;
        }
    }
    let model = config.model.as_deref().unwrap_or("antigravity-default");
    let mut sess = sessions.reset_provider_session(&key, &config.provider, model).await.unwrap();
    let _ = crate::telegram::session_init::initialize_session_if_needed(bot, msg, sessions, &mut sess, cli, config).await;
    Ok(())
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
    let cmd = args.split_whitespace().next().unwrap_or("");
    let topic_id = crate::telegram::get_topic_id(msg);

    if cmd == "/new" || cmd == "/reset" {
        handle_new_command(bot, msg, args, topic_id, config, sessions, cli, topic_cache).await?;
        return Ok(true);
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
