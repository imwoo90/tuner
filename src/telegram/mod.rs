//! # Telegram Bot Integration for Tuner
//!
//! This module implements the Teloxide-based Telegram Bot interface.
//! It polls message updates, filters allowed chats/users, maps chat IDs
//! to persistent agy conversation sessions, and streams real-time updates.

use teloxide::prelude::*;
use crate::config::CliConfig;
use crate::cli::antigravity::AntigravityCli;
use crate::cli::{AgentProvider, StreamEvent};
use std::sync::Arc;
use std::time::{Instant, Duration};

pub mod formatting;
#[cfg(test)]
pub mod formatting_tests;
pub mod reply;
#[cfg(test)]
pub mod reply_tests;
#[cfg(test)]
pub mod handler_tests;
pub mod commands;
pub mod cron_selector;
pub mod topic_cache;
pub mod stream;

pub(crate) use reply::build_reply_prompt;

pub(crate) fn get_topic_id(msg: &Message) -> Option<i64> {
    let is_topic = match &msg.kind {
        teloxide::types::MessageKind::Common(common) => common.is_topic_message,
        _ => false,
    };
    if is_topic {
        msg.thread_id.map(|t| t as i64)
    } else {
        None
    }
}

pub use topic_cache::{BotInfo, TopicNameCache};

async fn process_text(
    bot: &Bot,
    msg: &Message,
    text: &str,
    config: &CliConfig,
    sessions: &crate::session::manager::SessionManager,
    cli: &AntigravityCli,
    cron_manager: &crate::cron::manager::CronManager,
    topic_cache: &TopicNameCache,
) -> Result<(), teloxide::RequestError> {
    if commands::handle_commands(bot, msg, text, config, sessions, cli, cron_manager, topic_cache).await? {
        return Ok(());
    }

    let topic_id = get_topic_id(msg);
    let key = crate::session::key::SessionKey::telegram(msg.chat.id.0, topic_id);
    let model = config.model.as_deref().unwrap_or("antigravity-default");
    let (sess, _) = sessions.resolve_session(&key, &config.provider, model).await.unwrap();
    let sid = sess.get_session_id(&config.provider);

    let mut req = bot.send_message(msg.chat.id, "⏳ [우덕터] Processing...");
    if let Some(t) = msg.thread_id { req = req.message_thread_id(t); }
    let proc_msg = match req.await { Ok(m) => m, Err(_) => return Ok(()), };

    let prompt = if reply::has_media(msg) {
        let media_prompt = if text.is_empty() { "[INCOMING FILE]" } else { text };
        reply::prepend_reply_to_media(msg, media_prompt)
    } else {
        build_reply_prompt(msg, text)
    };

    let opt_sid = (!sid.is_empty()).then_some(&sid[..]);
    let stream_res = cli.send_streaming(&prompt, opt_sid, false, config.working_dir.clone()).await;

    match stream_res {
        Ok(stream) => {
            stream::consume_stream(bot, msg.chat.id, proc_msg.id, msg.thread_id, stream, sessions, sess, config).await?;
        }
        Err(e) => {
            let _ = bot.edit_message_text(msg.chat.id, proc_msg.id, format!("❌ Error: {}", e)).await;
        }
    }
    Ok(())
}

pub(crate) async fn handle_message(
    bot: Bot,
    msg: Message,
    config: Arc<CliConfig>,
    sessions: Arc<crate::session::manager::SessionManager>,
    cli: Arc<AntigravityCli>,
    cron_manager: Arc<crate::cron::manager::CronManager>,
    topic_cache: Arc<TopicNameCache>,
    bot_info: Arc<BotInfo>,
) -> Result<(), teloxide::RequestError> {
    let from_id = msg.from().map(|u| u.id.0 as i64).unwrap_or(0);
    let chat_id_raw = msg.chat.id.0;
    if let Some(target_chat_id) = msg.migrate_to_chat_id() {
        let _ = sessions.migrate_chat_id(chat_id_raw, target_chat_id.0).await;
        return Ok(());
    }

    match &msg.kind {
        teloxide::types::MessageKind::ForumTopicCreated(c) => {
            if let Some(tid) = msg.thread_id {
                topic_cache.insert(chat_id_raw, tid as i64, c.forum_topic_created.name.clone());
            }
            return Ok(());
        }
        teloxide::types::MessageKind::ForumTopicEdited(e) => {
            if let Some(tid) = msg.thread_id {
                if let Some(ref name) = e.forum_topic_edited.name {
                    topic_cache.insert(chat_id_raw, tid as i64, name.clone());
                }
            }
            return Ok(());
        }
        _ => {}
    }

    let ok = if msg.chat.is_group() || msg.chat.is_supergroup() {
        config.allowed_group_ids.contains(&chat_id_raw) && config.allowed_user_ids.contains(&from_id)
    } else {
        config.allowed_user_ids.contains(&from_id)
    };
    if !ok { return Ok(()); }

    let has_med = reply::has_media(&msg);
    let bot_username_str = bot_info.username.as_deref();
    let raw_text = msg.text().or(msg.caption()).unwrap_or("");
    let text = reply::strip_mention(&raw_text, bot_username_str);

    if !text.is_empty() || has_med {
        process_text(&bot, &msg, &text, &config, &sessions, &cli, &cron_manager, &topic_cache).await?;
    }
    Ok(())
}

fn build_sessions(path: std::path::PathBuf, cache: Arc<TopicNameCache>) -> crate::session::manager::SessionManager {
    let resolver_cache = cache.clone();
    crate::session::manager::SessionManager::new(path, 30, 4, false, "UTC".to_string(), None)
        .with_topic_resolver(Arc::new(move |cid, tid| resolver_cache.find_by_id(cid, tid)))
}

fn start_schedulers(
    bot: Bot,
    config: Arc<CliConfig>,
    sessions: Arc<crate::session::manager::SessionManager>,
    cli: Arc<AntigravityCli>,
    home: &str,
) -> Arc<crate::cron::manager::CronManager> {
    let scheduler = Arc::new(crate::heartbeat::scheduler::HeartbeatScheduler::new(
        config.clone(),
        sessions,
        cli.clone(),
    ));
    scheduler.start(bot.clone());

    let cron_path = std::path::PathBuf::from(home).join(".ductor").join("cron_jobs.json");
    let cron_manager = Arc::new(crate::cron::manager::CronManager::new(cron_path));
    let cron_scheduler = Arc::new(crate::cron::scheduler::CronScheduler::new(
        config,
        cron_manager.clone(),
        cli,
    ));
    cron_scheduler.start(bot);
    cron_manager
}

pub async fn run_bot(config: CliConfig) -> Result<(), String> {
    let token = std::env::var("TELEGRAM_TOKEN")
        .unwrap_or_else(|_| config.telegram_token.clone());
    if token.is_empty() {
        return Err("No Telegram token provided".to_string());
    }

    let bot = Bot::new(token);
    let me = bot.get_me().await.ok();
    let bot_username = me.and_then(|m| m.user.username);
    let bot_info = Arc::new(BotInfo { username: bot_username });

    let config_arc = Arc::new(config);
    let home = std::env::var("HOME").unwrap_or_else(|_| "/home/wimvm".to_string());
    let sessions_path = std::path::PathBuf::from(&home).join(".ductor").join("sessions.json");
    
    let topic_cache = Arc::new(TopicNameCache::new());
    let sessions = Arc::new(build_sessions(sessions_path, topic_cache.clone()));

    if let Ok(all_sessions) = sessions.list_all().await {
        for sess in all_sessions {
            if let (Some(tid), Some(tname)) = (sess.topic_id, sess.topic_name) {
                topic_cache.insert(sess.chat_id, tid, tname);
            }
        }
    }
    
    let cli = Arc::new(AntigravityCli::new((*config_arc).clone()));
    let cron_manager = start_schedulers(bot.clone(), config_arc.clone(), sessions.clone(), cli.clone(), &home);

    let handler = dptree::entry()
        .branch(Update::filter_message().endpoint(handle_message))
        .branch(Update::filter_callback_query().endpoint(handle_callback_query));

    let mut dispatcher = Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![config_arc, sessions, cli, cron_manager, topic_cache, bot_info])
        .build();

    dispatcher.dispatch().await;
    Ok(())
}

async fn handle_callback_query(
    bot: Bot,
    q: teloxide::types::CallbackQuery,
    config: Arc<CliConfig>,
    sessions: Arc<crate::session::manager::SessionManager>,
    cron_manager: Arc<crate::cron::manager::CronManager>,
) -> Result<(), teloxide::RequestError> {
    if let Some(ref data) = q.data {
        if data.starts_with("model:") {
            let model_name = &data["model:".len()..];
            if let Some(ref msg) = q.message {
                let chat_id = msg.chat.id.0;
                let topic_id = get_topic_id(msg);
                let key = crate::session::key::SessionKey::telegram(chat_id, topic_id);
                let default_model = config.model.as_deref().unwrap_or("antigravity-default");
                if let Ok((mut sess, _)) = sessions.resolve_session(&key, &config.provider, default_model).await {
                    sess.model = model_name.to_string();
                    let _ = sessions.update_session(&sess, 0.0, 0).await;
                    
                    let _ = bot.edit_message_text(
                        msg.chat.id,
                        msg.id,
                        format!("🤖 [우덕터] 세션의 LLM 모델이 `{}`(으)로 전환되었습니다.", model_name)
                    ).await;
                }
            }
            let _ = bot.answer_callback_query(q.id).await;
        } else if data.starts_with("crn:") {
            if let Some(ref msg) = q.message {
                let _ = cron_selector::handle_cron_callback(&bot, msg.chat.id, msg.id, data, &cron_manager).await;
            }
            let _ = bot.answer_callback_query(q.id).await;
        }
    }
    Ok(())
}
