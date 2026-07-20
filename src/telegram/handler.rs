use std::sync::Arc;
use teloxide::prelude::*;
use crate::config::CliConfig;
use crate::session::manager::SessionManager;
use crate::cli::antigravity::AntigravityCli;
use crate::cron::manager::CronManager;
use super::topic_cache::{BotInfo, TopicNameCache};
use super::{reply, commands, topic_cache, session_init, ask_helpers, get_topic_id, process_text_with_files};

fn auto_register_owner(from_id: i64) {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let path = std::path::PathBuf::from(&home).join(".tuner/config/config.json");
    if let Ok(c) = std::fs::read_to_string(&path) {
        if let Ok(mut val) = serde_json::from_str::<serde_json::Value>(&c) {
            if let Some(obj) = val.as_object_mut() {
                obj.insert("allowed_user_ids".to_string(), serde_json::json!([from_id]));
                if let Ok(pretty) = serde_json::to_string_pretty(&val) {
                    if std::fs::write(&path, pretty).is_ok() {
                        let restart = std::path::PathBuf::from(&home).join(".tuner/restart-requested");
                        let _ = std::fs::write(restart, "");
                    }
                }
            }
        }
    }
}

async fn validate_and_auth_message(
    bot: &Bot,
    msg: &Message,
    config: &CliConfig,
    sessions: &SessionManager,
    topic_cache: &TopicNameCache,
) -> Result<bool, teloxide::RequestError> {
    let from_id = msg.from().map(|u| u.id.0 as i64).unwrap_or(0);
    let chat_id = msg.chat.id.0;
    if let Some(to_chat) = msg.migrate_to_chat_id() {
        let _ = sessions.migrate_chat_id(chat_id, to_chat.0).await;
        return Ok(false);
    }
    if topic_cache::handle_forum_topic_events(msg, topic_cache, chat_id) {
        return Ok(false);
    }
    let mut ok = config.allowed_user_ids.contains(&from_id);

    if !ok && from_id != 0 && config.allowed_user_ids.is_empty() {
        println!("🤖 [tuner] First-time owner auto-registered! Telegram User ID: {}. Restarting...", from_id);
        let _ = bot.send_message(msg.chat.id, "🤖 Owner registered successfully! Restarting tuner daemon...").await;
        auto_register_owner(from_id);
        std::process::exit(0);
    }

    let is_group = msg.chat.is_group() || msg.chat.is_supergroup();
    if ok && is_group && !config.allowed_group_ids.contains(&chat_id) {
        eprintln!("⚠️ [tuner] Unauthorized group ID: {}", chat_id);
    }
    ok = ok && (!is_group || config.allowed_group_ids.contains(&chat_id));
    Ok(ok)
}

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

async fn handle_message_inner(
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
    if !validate_and_auth_message(&bot, &msg, &config, &sessions, &topic_cache).await? {
        return Ok(());
    }

    let raw_text = msg.text().or(msg.caption()).unwrap_or("");
    let text = reply::strip_mention(raw_text, bot_info.username.as_deref())
        .replace("/teamwork_preview", "/teamwork-preview")
        .replace("/grill_me", "/grill-me");

    route_media_message(
        bot,
        msg,
        text,
        config,
        sessions,
        cli,
        cron_manager,
        topic_cache,
        bot_info,
        media_group_manager,
    ).await
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
    let topic_id = get_topic_id(&msg);
    let key = crate::session::key::SessionKey::telegram(msg.chat.id.0, topic_id);
    let default_model = config.model.as_deref().unwrap_or("antigravity-default");
    
    let active_lang = sessions.resolve_session(&key, &config.provider, default_model).await
        .map(|(s, _)| s.language)
        .ok().flatten()
        .or_else(|| config.language.clone())
        .unwrap_or_else(|| "en".to_string());

    let fut = crate::i18n::TASK_ACTIVE_LANG.scope(active_lang, async move {
        handle_message_inner(bot, msg, config, sessions, cli, cron_manager, topic_cache, bot_info, media_group_manager).await
    });
    if cfg!(test) { fut.await } else { tokio::spawn(fut); Ok(()) }
}
