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
    let res = if let Some(group_id) = media_group_id {
        handle_media_group_message(bot, msg, group_id, deps, media_group_manager).await
    } else if reply::has_media(&msg) {
        handle_single_media_message(bot, msg, text, &deps).await
    } else if !text.is_empty() {
        handle_pure_text_message(bot, msg, text, &deps).await
    } else {
        Ok(())
    };
    crate::upgrade::record_user_activity();
    res
}

pub(crate) async fn resolve_active_lang(
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
    let stripped = reply::strip_mention(raw_text, bot_username);
    let cleaned = if stripped.trim_start().starts_with('/') {
        let (cmd, args) = super::commands_registry::split_slash_command(&stripped);
        if args.is_empty() { cmd.to_string() } else { format!("{} {}", cmd, args) }
    } else {
        stripped
    };
    cleaned
        .replace("/teamwork_preview", "/teamwork")
        .replace("/teamwork-preview", "/teamwork")
        .replace("/grill_me", "/grill-me")
        .replace("/security_review", "/security-review")
        .replace("/deep_research", "/deep-research")
        .replace("/code_review", "/code-review")
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

tokio::task_local! {
    pub(crate) static LOCKED_TOPIC_ID: Result<Option<i64>, String>;
}

pub(crate) fn resolve_target_topic(
    msg: &Message,
    args: &str,
    topic_cache: &TopicNameCache,
    default_tid: Option<i64>,
) -> Result<Option<i64>, String> {
    let trimmed = args.trim();
    let raw_cmd = trimmed.split_whitespace().next().unwrap_or("");
    let rest = trimmed[raw_cmd.len()..].trim();
    if rest.is_empty() {
        return Ok(default_tid);
    }
    topic_cache.find_by_name(msg.chat.id.0, rest).map(Some).ok_or_else(|| rest.to_string())
}

pub(crate) async fn is_topic_busy_or_asking(
    sessions: &SessionManager,
    cli: &AntigravityCli,
    chat_id: i64,
    topic_id: Option<i64>,
) -> bool {
    if cli.is_topic_turn_in_flight(chat_id, topic_id)
        || cli.sessions.boot_cancels.lock().await.contains_key(&(chat_id, topic_id))
        || crate::cli::claude::is_running(chat_id, topic_id, None).await
        || crate::cli::claude::has_pending_ask(chat_id, topic_id, None).await
        || crate::cli::codex::is_running(chat_id, topic_id, None).await
        || crate::cli::codex::has_pending_ask(chat_id, topic_id, None).await
    {
        return true;
    }
    let key = crate::session::key::SessionKey::telegram(chat_id, topic_id);
    if let Ok(Some(sess)) = sessions.get_active(&key).await {
        for ps in sess.provider_sessions.values() {
            if !ps.session_id.is_empty() && cli.is_pty_busy_or_asking(&ps.session_id, chat_id, topic_id).await {
                return true;
            }
        }
    }
    false
}

pub(crate) fn resolve_command_target_topic(
    msg: &Message,
    text: &str,
    topic_cache: &TopicNameCache,
    msg_topic_id: Option<i64>,
) -> Result<Option<i64>, String> {
    let (cmd, _) = super::commands_registry::split_slash_command(text.trim());
    if matches!(cmd, "/new" | "/reset") {
        return resolve_target_topic(msg, text, topic_cache, msg_topic_id);
    }
    Ok(msg_topic_id)
}

async fn dispatch_locked_message(
    bot: Bot,
    msg: Message,
    text: String,
    topic_id: Option<i64>,
    deps: MediaGroupDeps,
    media_group_manager: Arc<MediaGroupManager>,
) -> Result<(), teloxide::RequestError> {
    let lock_pool = deps.sessions.lock_pool.clone();
    let target_res = resolve_command_target_topic(&msg, &text, &deps.topic_cache, topic_id);
    let lock_tid = *target_res.as_ref().unwrap_or(&topic_id);
    let lock = lock_pool.get((msg.chat.id.0, lock_tid));
    let _guard = lock.lock().await;
    let cur_gid = msg.media_group_id().map(|id| id.to_string());
    media_group_manager
        .flush_other_groups_for_topic(&bot, msg.chat.id.0, lock_tid, cur_gid.as_deref(), &deps)
        .await?;
    LOCKED_TOPIC_ID.scope(target_res, route_media_message(bot, msg, text, deps, media_group_manager)).await
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
    crate::upgrade::record_user_activity();

    let topic_id = get_topic_id(&msg);
    let active_lang = resolve_active_lang(&sessions, &config, msg.chat.id.0, topic_id).await;

    let raw_text = msg.text().or(msg.caption()).unwrap_or("");
    let text = normalize_command_text(raw_text, bot_info.username.as_deref());
    if !text.starts_with("/provider") {
        super::commands_registry::sync_chat_commands_for_topic(&bot, &msg, &sessions, &config).await;
    }
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

    let fut = crate::i18n::TASK_ACTIVE_LANG.scope(active_lang, async move {
        dispatch_locked_message(bot, msg, text, topic_id, deps, media_group_manager).await
    });
    if cfg!(test) { fut.await } else { tokio::spawn(fut); Ok(()) }
}
