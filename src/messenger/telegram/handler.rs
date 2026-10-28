//! # Main Message Ingress Dispatcher
//!
//! ## Overview
//! The central routing entry point for incoming Telegram updates. Parses messages, extracts media group
//! attachments, routes callback queries, and executes command or chat stream pipelines.
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
use super::media_group::{MediaGroupDeps, MediaGroupManager};
use super::{reply, get_topic_id, process_text_with_files};

async fn handle_media_group_message(
    bot: Bot,
    msg: Message,
    group_id: String,
    deps: MediaGroupDeps,
    media_group_manager: Arc<MediaGroupManager>,
) -> Result<(), teloxide::RequestError> {
    media_group_manager.add_message(bot, msg, group_id, deps).await;
    Ok(())
}

fn default_media_prompt(is_video_note: bool) -> &'static str {
    if is_video_note {
        "[Video note attached. Process or inspect the attached video file (e.g., via `view_file` or `tools/media_tools/process_video.py`) and respond directly to the user's request.]"
    } else {
        "[Voice message attached. Listen to or transcribe the attached audio file (e.g., via `view_file` or `tools/media_tools/transcribe_audio.py`) and respond directly to the user's spoken request.]"
    }
}

async fn handle_single_media_message(
    bot: Bot,
    msg: Message,
    text: String,
    deps: &MediaGroupDeps,
) -> Result<(), teloxide::RequestError> {
    let dest_dir = deps.config.working_dir.join("telegram_files");
    let downloaded = reply::download_telegram_media(&bot, &msg, &dest_dir).await.ok().flatten();
    if text.is_empty() && downloaded.is_none() {
        return Ok(());
    }
    let files: Vec<String> = downloaded.into_iter().collect();
    let is_voice_or_video_note = msg.voice().is_some() || msg.video_note().is_some();
    let effective_text = if text.is_empty() && is_voice_or_video_note {
        default_media_prompt(msg.video_note().is_some())
    } else {
        text.as_str()
    };
    process_text_with_files(
        &bot,
        &msg,
        (effective_text, &files),
        &deps.config,
        &deps.sessions,
        &deps.cli,
        (&deps.cron_manager, &deps.topic_cache),
    ).await
}

async fn handle_pure_text_message(
    bot: Bot,
    msg: Message,
    text: String,
    deps: &MediaGroupDeps,
) -> Result<(), teloxide::RequestError> {
    process_text_with_files(
        &bot,
        &msg,
        (&text, &[]),
        &deps.config,
        &deps.sessions,
        &deps.cli,
        (&deps.cron_manager, &deps.topic_cache),
    ).await
}

async fn route_media_message(
    bot: Bot,
    msg: Message,
    text: String,
    deps: MediaGroupDeps,
    media_group_manager: Arc<MediaGroupManager>,
) -> Result<(), teloxide::RequestError> {
    let media_group_id = msg.media_group_id().map(|s| s.to_string());
    if let Some(group_id) = media_group_id {
        return handle_media_group_message(bot, msg, group_id, deps, media_group_manager).await;
    }

    if reply::has_media(&msg) {
        handle_single_media_message(bot, msg, text, &deps).await
    } else if !text.is_empty() {
        handle_pure_text_message(bot, msg, text, &deps).await
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
    sessions.get_active(&key).await
        .ok().flatten()
        .and_then(|s| s.language)
        .or_else(|| config.language.clone())
        .unwrap_or_else(|| "en".to_string())
}

pub(crate) fn normalize_command_text(raw_text: &str, bot_username: Option<&str>) -> String {
    reply::strip_mention(raw_text, bot_username)
        .replace("/teamwork_preview", "/teamwork-preview")
        .replace("/grill_me", "/grill-me")
}

async fn dispatch_lock_free(
    bot: Bot,
    msg: Message,
    text: String,
    deps: MediaGroupDeps,
) {
    if let Err(e) = super::commands::handle_commands(
        &bot,
        &msg,
        &text,
        &deps.config,
        &deps.sessions,
        &deps.cli,
        (&deps.cron_manager, &deps.topic_cache),
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
    aux: (Arc<CronManager>, Arc<TopicNameCache>),
    meta: (Arc<BotInfo>, Arc<MediaGroupManager>),
) -> Result<(), teloxide::RequestError> {
    let (cron_manager, topic_cache) = aux;
    let (bot_info, media_group_manager) = meta;
    println!("🤖 [tuner] handle_message: received update from chat {}, text: {:?}", msg.chat.id, msg.text().or(msg.caption()));
    if !super::auth::validate_and_auth_message(&bot, &msg, &config, &sessions, &topic_cache).await? {
        return Ok(());
    }

    let topic_id = get_topic_id(&msg);
    let active_lang = resolve_active_lang(&sessions, &config, msg.chat.id.0, topic_id).await;

    let raw_text = msg.text().or(msg.caption()).unwrap_or("");
    let text = normalize_command_text(raw_text, bot_info.username.as_deref());
    let deps = MediaGroupDeps {
        config,
        sessions: sessions.clone(),
        cli,
        cron_manager,
        topic_cache,
        bot_username: bot_info.username.clone(),
    };

    if super::commands_registry::is_lock_free_command(&text) {
        let fut = crate::i18n::TASK_ACTIVE_LANG.scope(active_lang, async move {
            dispatch_lock_free(bot, msg, text, deps).await;
        });
        if cfg!(test) { fut.await; } else { tokio::spawn(fut); }
        return Ok(());
    }

    let lock_pool = sessions.lock_pool.clone();
    let fut = crate::i18n::TASK_ACTIVE_LANG.scope(active_lang, async move {
        let lock = lock_pool.get((msg.chat.id.0, topic_id));
        let _guard = lock.lock().await;
        route_media_message(bot, msg, text, deps, media_group_manager).await
    });
    if cfg!(test) { fut.await } else { tokio::spawn(fut); Ok(()) }
}
