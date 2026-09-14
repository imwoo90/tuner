//! # Main Message Ingress Dispatcher
//!
//! ## Overview
//! The central routing entry point for incoming Telegram updates. Parses messages, extracts media group
//! attachments, routes callback queries, and executes command or chat stream pipelines.
//!
//! ## Collaboration Graph
//! - Binds directly to the Teloxide listener event loop.
//! - Dispatches commands to [`super::commands::handle_commands`].
//! - Routes callbacks to [`super::callbacks::handle_callback_query`].
//! - Downloads photos/files via [`super::reply::download_telegram_media`].
//!
//! ## Search Tags
//! #ingress-handler, #telegram-dispatcher, #event-router, #media-downloads

use std::sync::Arc;
use teloxide::prelude::*;
use crate::config::CliConfig;
use crate::session::manager::SessionManager;
use crate::cli::antigravity::AntigravityCli;
use crate::cron::manager::CronManager;
use super::topic_cache::{BotInfo, TopicNameCache};
use super::{reply, get_topic_id, process_text_with_files};



async fn handle_media_group_message(
    bot: Bot,
    msg: Message,
    group_id: String,
    config: Arc<CliConfig>,
    sessions: Arc<SessionManager>,
    cli: Arc<AntigravityCli>,
    cron_manager: Arc<CronManager>,
    topic_cache: Arc<TopicNameCache>,
    bot_info: Arc<BotInfo>,
    media_group_manager: Arc<super::media_group::MediaGroupManager>,
) -> Result<(), teloxide::RequestError> {
    media_group_manager.add_message(
        bot.clone(),
        msg.clone(),
        group_id.clone(),
        config.clone(),
        sessions.clone(),
        cli.clone(),
        cron_manager.clone(),
        topic_cache.clone(),
        bot_info.clone(),
    ).await;
    Ok(())
}

async fn handle_single_media_message(
    bot: Bot,
    msg: Message,
    text: String,
    config: Arc<CliConfig>,
    sessions: Arc<SessionManager>,
    cli: Arc<AntigravityCli>,
    cron_manager: Arc<CronManager>,
    topic_cache: Arc<TopicNameCache>,
) -> Result<(), teloxide::RequestError> {
    if text.is_empty() {
        let dest_dir = config.working_dir.join("telegram_files");
        match reply::download_telegram_media(&bot, &msg, &dest_dir).await {
            Ok(Some(relative_path)) => {
                let key = crate::session::key::SessionKey::telegram(msg.chat.id.0, get_topic_id(&msg));
                let default_model = config.model.as_deref().unwrap_or("antigravity-default");
                if let Ok((mut sess, _)) = sessions.resolve_session(&key, &config.provider, default_model).await {
                    sess.pending_attachments.push(relative_path);
                    let _ = sessions.preserve_session_identity(&sess).await;
                }
            }
            _ => {}
        }
    } else {
        let dest_dir = config.working_dir.join("telegram_files");
        let mut files = Vec::new();
        if let Ok(Some(relative_path)) = reply::download_telegram_media(&bot, &msg, &dest_dir).await {
            files.push(relative_path);
        }
        process_text_with_files(
            &bot,
            &msg,
            &text,
            &files,
            &config,
            &sessions,
            &cli,
            &cron_manager,
            &topic_cache,
        ).await?;
    }
    Ok(())
}

async fn handle_pure_text_message(
    bot: Bot,
    msg: Message,
    text: String,
    config: Arc<CliConfig>,
    sessions: Arc<SessionManager>,
    cli: Arc<AntigravityCli>,
    cron_manager: Arc<CronManager>,
    topic_cache: Arc<TopicNameCache>,
) -> Result<(), teloxide::RequestError> {
    let key = crate::session::key::SessionKey::telegram(msg.chat.id.0, get_topic_id(&msg));
    let default_model = config.model.as_deref().unwrap_or("antigravity-default");
    let mut files = Vec::new();
    if let Ok((mut sess, _)) = sessions.resolve_session(&key, &config.provider, default_model).await {
        if !sess.pending_attachments.is_empty() {
            files = sess.pending_attachments.clone();
            sess.pending_attachments.clear();
            let _ = sessions.preserve_session_identity(&sess).await;
        }
    }
    process_text_with_files(
        &bot,
        &msg,
        &text,
        &files,
        &config,
        &sessions,
        &cli,
        &cron_manager,
        &topic_cache,
    ).await?;
    Ok(())
}

async fn route_media_message(
    bot: Bot,
    msg: Message,
    text: String,
    config: Arc<CliConfig>,
    sessions: Arc<SessionManager>,
    cli: Arc<AntigravityCli>,
    cron_manager: Arc<CronManager>,
    topic_cache: Arc<TopicNameCache>,
    bot_info: Arc<BotInfo>,
    media_group_manager: Arc<super::media_group::MediaGroupManager>,
) -> Result<(), teloxide::RequestError> {
    let media_group_id = msg.media_group_id().map(|s| s.to_string());
    if let Some(group_id) = media_group_id {
        return handle_media_group_message(
            bot, msg, group_id, config, sessions, cli, cron_manager, topic_cache, bot_info, media_group_manager
        ).await;
    }

    if reply::has_media(&msg) {
        handle_single_media_message(bot, msg, text, config, sessions, cli, cron_manager, topic_cache).await
    } else if !text.is_empty() {
        handle_pure_text_message(bot, msg, text, config, sessions, cli, cron_manager, topic_cache).await
    } else {
        Ok(())
    }
}

async fn resolve_active_lang(
    sessions: &SessionManager,
    config: &CliConfig,
    chat_id: i64,
    topic_id: Option<i64>,
) -> String {
    let key = crate::session::key::SessionKey::telegram(chat_id, topic_id);
    let default_model = config.model.as_deref().unwrap_or("antigravity-default");
    sessions.resolve_session(&key, &config.provider, default_model).await
        .map(|(s, _)| s.language)
        .ok().flatten()
        .or_else(|| config.language.clone())
        .unwrap_or_else(|| "en".to_string())
}

fn normalize_command_text(raw_text: &str, bot_username: Option<&str>) -> String {
    reply::strip_mention(raw_text, bot_username)
        .replace("/teamwork_preview", "/teamwork-preview")
        .replace("/grill_me", "/grill-me")
}

async fn dispatch_lock_free(
    bot: Bot,
    msg: Message,
    text: String,
    config: Arc<CliConfig>,
    sessions: Arc<SessionManager>,
    cli: Arc<AntigravityCli>,
    cron_manager: Arc<CronManager>,
    topic_cache: Arc<TopicNameCache>,
) {
    if let Err(e) = super::commands::handle_commands(
        &bot, &msg, &text, &config, &sessions, &cli, &cron_manager, &topic_cache,
    ).await {
        eprintln!("❌ [tuner] Error handling lock-free command '{}' (chat: {}, thread: {:?}): {:?}", text, msg.chat.id, msg.thread_id.map(|t| t.0.0), e);
    }
}

pub(crate) async fn handle_message(
    bot: Bot,
    msg: Message,
    config: Arc<CliConfig>,
    sessions: Arc<SessionManager>,
    cli: Arc<AntigravityCli>,
    cron_manager: Arc<CronManager>,
    topic_cache: Arc<TopicNameCache>,
    bot_info: Arc<BotInfo>,
    media_group_manager: Arc<super::media_group::MediaGroupManager>,
) -> Result<(), teloxide::RequestError> {
    println!("🤖 [tuner] handle_message: received update from chat {}, text: {:?}", msg.chat.id, msg.text().or(msg.caption()));
    if !super::auth::validate_and_auth_message(&bot, &msg, &config, &sessions, &topic_cache).await? {
        return Ok(());
    }

    let topic_id = get_topic_id(&msg);
    let active_lang = resolve_active_lang(&sessions, &config, msg.chat.id.0, topic_id).await;

    let raw_text = msg.text().or(msg.caption()).unwrap_or("");
    let text = normalize_command_text(raw_text, bot_info.username.as_deref());

    if super::commands_registry::is_lock_free_command(&text) {
        let fut = crate::i18n::TASK_ACTIVE_LANG.scope(active_lang, async move {
            dispatch_lock_free(bot, msg, text, config, sessions, cli, cron_manager, topic_cache).await;
        });
        if cfg!(test) { fut.await; } else { tokio::spawn(fut); }
        return Ok(());
    }

    let lock_pool = sessions.lock_pool.clone();
    let fut = crate::i18n::TASK_ACTIVE_LANG.scope(active_lang, async move {
        let lock = lock_pool.get((msg.chat.id.0, topic_id));
        let _guard = lock.lock().await;
        route_media_message(
            bot, msg, text, config, sessions, cli, cron_manager, topic_cache, bot_info, media_group_manager,
        ).await
    });
    if cfg!(test) { fut.await } else { tokio::spawn(fut); Ok(()) }
}
