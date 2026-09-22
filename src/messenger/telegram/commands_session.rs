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

fn resolve_target_topic(
    msg: &Message,
    args: &str,
    topic_cache: &super::TopicNameCache,
    default_tid: Option<i64>,
) -> Result<Option<i64>, String> {
    let parts: Vec<&str> = args.split_whitespace().collect();
    if parts.len() > 1 {
        let name = parts[1];
        if let Some(resolved_tid) = topic_cache.find_by_name(msg.chat.id.0, name) {
            Ok(Some(resolved_tid))
        } else {
            Err(name.to_string())
        }
    } else {
        Ok(default_tid)
    }
}

async fn collect_handover_context(
    config: &CliConfig,
    sessions: &crate::session::manager::SessionManager,
    cli: &AntigravityCli,
    topic_cache: &super::TopicNameCache,
    chat_id: i64,
    topic_id: Option<i64>,
) -> Option<super::handover::TopicHandoverContext> {
    let key = crate::session::key::SessionKey::telegram(chat_id, topic_id);
    let mut old_sid = String::new();
    let mut topic_name = None;
    let mut prior_summary = None;
    let mut past_ids = Vec::new();

    if let Ok(Some(existing_sess)) = sessions.get_active(&key).await {
        old_sid = existing_sess.get_session_id(&config.provider);
        past_ids = existing_sess.get_past_session_ids(&config.provider);
        if old_sid.is_empty() {
            if let Some(last_id) = past_ids.last() {
                old_sid = last_id.clone();
            }
        }
        topic_name = existing_sess.topic_name.clone();
        prior_summary = existing_sess.last_handover_summary.clone();
        if !old_sid.is_empty() {
            cli.sessions.terminate(&old_sid).await;
        }
    }

    if topic_id.is_none() {
        return None;
    }

    if topic_name.is_none() {
        if let Some(tid) = topic_id {
            topic_name = topic_cache.find_by_id(chat_id, tid);
        }
    }

    super::handover::build_topic_handover(
        &config.working_dir,
        &old_sid,
        topic_id,
        topic_name,
        prior_summary.as_deref(),
        &past_ids,
    )
}

pub(crate) async fn handle_new_command(
    bot: &Bot,
    msg: &Message,
    args: &str,
    topic_id: Option<i64>,
    config: &CliConfig,
    sessions: &crate::session::manager::SessionManager,
    cli: &AntigravityCli,
    topic_cache: &super::TopicNameCache,
) -> Result<(), teloxide::RequestError> {
    let target_topic = match resolve_target_topic(msg, args, topic_cache, topic_id) {
        Ok(tid) => tid,
        Err(name) => {
            let _ = send_reply(bot, msg, t!("bot.unknown_topic", name = name)).await;
            return Ok(());
        }
    };

    let handover_context = collect_handover_context(
        config, sessions, cli, topic_cache, msg.chat.id.0, target_topic,
    ).await;

    let key = crate::session::key::SessionKey::telegram(msg.chat.id.0, target_topic);
    let model = config.model.as_deref().unwrap_or("antigravity-default");
    let mut sess = sessions.reset_provider_session(&key, &config.provider, model).await.unwrap();

    let startup_prompt = if let Some(ref handover) = handover_context {
        sess.last_handover_summary = Some(format!(
            "Objective: {}\nDecisions: {}\nProgress: {}",
            handover.topic_objective,
            handover.agreed_decisions.join("; "),
            handover.current_progress
        ));
        let _ = sessions.preserve_session_identity(&sess).await;
        Some(handover.handover_prompt.as_str())
    } else {
        None
    };

    let _ = crate::telegram::session_init::initialize_session_with_prompt(
        bot, msg, sessions, &mut sess, cli, config, startup_prompt,
    ).await;
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
    let raw_cmd = args.split_whitespace().next().unwrap_or("");
    let cmd = raw_cmd.split('@').next().unwrap_or(raw_cmd);
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
