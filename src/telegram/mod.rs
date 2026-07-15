//! # Telegram Bot Integration for Tuner
//!
//! This module implements the Teloxide-based Telegram Bot interface.
//! It polls message updates, filters allowed chats/users, maps chat IDs
//! to persistent agy conversation sessions, and streams real-time updates.

use teloxide::prelude::*;
use crate::config::CliConfig;
use crate::cli::antigravity::AntigravityCli;
use crate::cli::AgentProvider;
use crate::session::manager::SessionManager;
use crate::session::data::SessionData;
use std::sync::Arc;

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
pub mod transport;

pub(crate) use reply::build_reply_prompt;
pub use transport::TelegramTransport;


pub(crate) fn get_topic_id(msg: &Message) -> Option<i64> {
    match &msg.kind {
        teloxide::types::MessageKind::Common(c) if c.is_topic_message => msg.thread_id.map(|t| t as i64),
        _ => None,
    }
}

pub use topic_cache::{BotInfo, TopicNameCache};

fn parse_model_directive(text: &str) -> (Option<String>, &str) {
    let t = text.trim();
    if t.starts_with("@model ") {
        let p: Vec<&str> = t["@model ".len()..].splitn(2, ' ').collect();
        if !p.is_empty() { return (Some(p[0].to_string()), p.get(1).unwrap_or(&"").trim()); }
    } else if t.starts_with('@') {
        let dir = t.split_whitespace().next().unwrap_or("");
        let m = &dir[1..];
        if ["opus", "sonnet", "haiku", "gpt-4o", "gpt-4-turbo"].contains(&m) {
            return (Some(m.to_string()), t[dir.len()..].trim());
        }
    }
    (None, t)
}

async fn run_cli_stream(
    bot: &Bot,
    msg: &Message,
    prompt: &str,
    sid: &str,
    cli: &AntigravityCli,
    sessions: &SessionManager,
    sess: SessionData,
    config: &CliConfig,
) -> Result<(), teloxide::RequestError> {
    let mut req = bot.send_message(msg.chat.id, crate::t!("bot.processing"));
    if let Some(t) = msg.thread_id { req = req.message_thread_id(t); }
    let proc_msg_opt = req.await.ok();

    let opt_sid = (!sid.is_empty()).then_some(sid);
    let stream_res = cli.send_streaming(prompt, opt_sid, false, config.working_dir.clone()).await;
    match stream_res {
        Ok(stream) => {
            let proc_msg_id = proc_msg_opt.map(|m| m.id).unwrap_or(teloxide::types::MessageId(0));
            stream::consume_stream(bot, msg.chat.id, proc_msg_id, msg.thread_id, stream, sessions, sess, config).await?;
        }
        Err(e) => {
            eprintln!("CLI STREAM ERROR: {:?}", e);
            if let Some(proc_msg) = proc_msg_opt {
                let _ = bot.edit_message_text(msg.chat.id, proc_msg.id, format!("❌ Error: {}", e)).await;
            }
        }
    }
    Ok(())
}

async fn process_text(
    bot: &Bot,
    msg: &Message,
    text: &str,
    config: &CliConfig,
    sessions: &SessionManager,
    cli: &AntigravityCli,
    cron_manager: &crate::cron::manager::CronManager,
    topic_cache: &TopicNameCache,
) -> Result<(), teloxide::RequestError> {
    if commands::handle_commands(bot, msg, text, config, sessions, cli, cron_manager, topic_cache).await? {
        return Ok(());
    }

    let (model_override, current_text) = parse_model_directive(text);
    let topic_id = get_topic_id(msg);
    let key = crate::session::key::SessionKey::telegram(msg.chat.id.0, topic_id);
    let mut model = config.model.as_deref().unwrap_or("antigravity-default").to_string();
    if let Some(ref m) = model_override { model = m.clone(); }

    let (mut sess, _) = sessions.resolve_session(&key, &config.provider, &model).await.unwrap();
    if let Some(ref m) = model_override {
        sess.model = m.clone();
        let _ = sessions.update_session(&sess, 0.0, 0).await;
        if current_text.is_empty() {
            let mut req = bot.send_message(msg.chat.id, format!("Next message will use {}", m));
            if let Some(t) = msg.thread_id { req = req.message_thread_id(t); }
            let _ = req.await;
            return Ok(());
        }
    }

    let prompt = if reply::has_media(msg) {
        reply::prepend_reply_to_media(msg, if current_text.is_empty() { "[INCOMING FILE]" } else { current_text })
    } else {
        build_reply_prompt(msg, current_text)
    };

    run_cli_stream(bot, msg, &prompt, &sess.get_session_id(&config.provider), cli, sessions, sess, config).await
}

pub(crate) async fn handle_message(
    bot: Bot,
    msg: Message,
    config: Arc<CliConfig>,
    sessions: Arc<SessionManager>,
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
            if let Some(tid) = msg.thread_id { topic_cache.insert(chat_id_raw, tid as i64, c.forum_topic_created.name.clone()); }
            return Ok(());
        }
        teloxide::types::MessageKind::ForumTopicEdited(e) => {
            if let Some(tid) = msg.thread_id {
                if let Some(ref name) = e.forum_topic_edited.name { topic_cache.insert(chat_id_raw, tid as i64, name.clone()); }
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
    if ok {
        let has_med = reply::has_media(&msg);
        let text = reply::strip_mention(msg.text().or(msg.caption()).unwrap_or(""), bot_info.username.as_deref());
        if !text.is_empty() || has_med {
            process_text(&bot, &msg, &text, &config, &sessions, &cli, &cron_manager, &topic_cache).await?;
        }
    }
    Ok(())
}

fn build_sessions(path: std::path::PathBuf, cache: Arc<TopicNameCache>) -> SessionManager {
    SessionManager::new(path, 30, 4, false, "UTC".to_string(), None)
        .with_topic_resolver(Arc::new(move |c, t| cache.find_by_id(c, t)))
}
fn start_schedulers(
    bot: Bot,
    config: Arc<CliConfig>,
    sessions: Arc<SessionManager>,
    cli: Arc<AntigravityCli>,
    home: &str,
) -> Arc<crate::cron::manager::CronManager> {
    let bus = Arc::new(crate::bus::bus::MessageBus::new());
    bus.register_transport(Arc::new(TelegramTransport::new(bot.clone())));
    Arc::new(crate::heartbeat::scheduler::HeartbeatScheduler::new(config.clone(), sessions, cli.clone(), bus.clone())).start();
    let cron_manager = Arc::new(crate::cron::manager::CronManager::new(std::path::PathBuf::from(home).join(".ductor/cron_jobs.json")));
    Arc::new(crate::cron::scheduler::CronScheduler::new(config.clone(), cron_manager.clone(), cli, bus)).start();
    let clean = Arc::new(crate::cleanup::observer::CleanupObserver::new(config.cleanup.clone(), config.working_dir.join("telegram_files"), config.working_dir.join("output_to_user")));
    tokio::spawn(async move { clean.start().await; });
    cron_manager
}

pub async fn run_bot(config: CliConfig) -> Result<(), String> {
    let token = std::env::var("TELEGRAM_TOKEN").unwrap_or_else(|_| config.telegram_token.clone());
    if token.is_empty() { return Err("No Telegram token provided".to_string()); }

    let bot = Bot::new(token);
    let me = bot.get_me().await.ok();
    let bot_info = Arc::new(BotInfo { username: me.and_then(|m| m.user.username) });
    let config_arc = Arc::new(config);
    let home = std::env::var("HOME").unwrap_or_else(|_| "/home/wimvm".to_string());
    
    let topic_cache = Arc::new(TopicNameCache::new());
    let sessions = Arc::new(build_sessions(std::path::PathBuf::from(&home).join(".ductor/sessions.json"), topic_cache.clone()));

    if let Ok(all) = sessions.list_all().await {
        for s in all {
            if let (Some(tid), Some(tname)) = (s.topic_id, s.topic_name) { topic_cache.insert(s.chat_id, tid, tname); }
        }
    }
    
    let cli = Arc::new(AntigravityCli::new((*config_arc).clone()));
    let cron_manager = start_schedulers(bot.clone(), config_arc.clone(), sessions.clone(), cli.clone(), &home);
    let handler = dptree::entry()
        .branch(Update::filter_message().endpoint(handle_message))
        .branch(Update::filter_callback_query().endpoint(handle_callback_query));

    Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![config_arc, sessions, cli, cron_manager, topic_cache, bot_info])
        .build().dispatch().await;
    Ok(())
}

async fn handle_callback_query(
    bot: Bot,
    q: teloxide::types::CallbackQuery,
    config: Arc<CliConfig>,
    sessions: Arc<SessionManager>,
    cron_manager: Arc<crate::cron::manager::CronManager>,
) -> Result<(), teloxide::RequestError> {
    if let Some(ref data) = q.data {
        if data.starts_with("model:") {
            let model_name = &data["model:".len()..];
            if let Some(ref msg) = q.message {
                let key = crate::session::key::SessionKey::telegram(msg.chat.id.0, get_topic_id(msg));
                let default_model = config.model.as_deref().unwrap_or("antigravity-default");
                if let Ok((mut sess, _)) = sessions.resolve_session(&key, &config.provider, default_model).await {
                    sess.model = model_name.to_string();
                    let _ = sessions.update_session(&sess, 0.0, 0).await;
                    let _ = bot.edit_message_text(msg.chat.id, msg.id, crate::t!("bot.model_switch_success", model = model_name)).await;
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
