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
pub mod commands;
pub mod cron_selector;

pub(crate) use reply::build_reply_prompt;

async fn handle_stream_result(
    bot: &Bot,
    chat_id: ChatId,
    msg_id: teloxide::types::MessageId,
    thread_id: Option<i32>,
    resp: crate::cli::CliResponse,
) -> Result<Option<String>, teloxide::RequestError> {
    let mut last_session_id = None;
    if let Some(ref sid) = resp.session_id {
        last_session_id = Some(sid.clone());
    }
    let raw_text = if resp.is_error {
        let code = resp.returncode.unwrap_or(1);
        let error_msg = if !resp.stderr.is_empty() { &resp.stderr } else { &resp.result };
        crate::cli::antigravity::error_parser::parse_cli_error(error_msg, code)
    } else {
        resp.result
    };

    let html_text = formatting::markdown_to_telegram_html(&raw_text);
    let chunks = formatting::split_html_message(&html_text, 4000);

    for (i, chunk) in chunks.iter().enumerate() {
        if i == 0 {
            let _ = bot.edit_message_text(chat_id, msg_id, chunk)
                .parse_mode(teloxide::types::ParseMode::Html)
                .await;
        } else {
            let mut req = bot.send_message(chat_id, chunk)
                .parse_mode(teloxide::types::ParseMode::Html);
            if let Some(tid) = thread_id {
                req = req.message_thread_id(tid);
            }
            let _ = req.await;
        }
    }
    Ok(last_session_id)
}

async fn consume_stream(
    bot: &Bot,
    chat_id: ChatId,
    msg_id: teloxide::types::MessageId,
    thread_id: Option<i32>,
    mut stream: futures::stream::BoxStream<'_, StreamEvent>,
    sessions: &crate::session::manager::SessionManager,
    session_data: crate::session::data::SessionData,
    config: &CliConfig,
) -> Result<(), teloxide::RequestError> {
    let mut last_edit = Instant::now();
    let mut last_session_id = None;
    
    use futures::StreamExt;
    while let Some(event) = stream.next().await {
        match event {
            StreamEvent::TextDelta(delta) => {
                if Instant::now().duration_since(last_edit) >= Duration::from_secs(2) {
                    let _ = bot.edit_message_text(chat_id, msg_id, &delta).await;
                    last_edit = Instant::now();
                }
            }
            StreamEvent::Result(resp) => {
                if let Ok(Some(sid)) = handle_stream_result(bot, chat_id, msg_id, thread_id, resp).await {
                    last_session_id = Some(sid);
                }
            }
        }
    }

    if let Some(sid) = last_session_id {
        let mut updated = session_data;
        updated.set_session_id(&config.provider, &sid);
        let _ = sessions.update_session(&updated, 0.0, 0).await;
    }
    Ok(())
}

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

async fn process_text(
    bot: &Bot,
    msg: &Message,
    text: &str,
    config: &CliConfig,
    sessions: &crate::session::manager::SessionManager,
    cli: &AntigravityCli,
    cron_manager: &crate::cron::manager::CronManager,
) -> Result<(), teloxide::RequestError> {
    if commands::handle_commands(bot, msg, text, config, sessions, cli, cron_manager).await? {
        return Ok(());
    }

    let topic_id = get_topic_id(msg);
    let key = crate::session::key::SessionKey::telegram(msg.chat.id.0, topic_id);
    let model = config.model.as_deref().unwrap_or("antigravity-default");
    let (sess, _) = sessions.resolve_session(&key, &config.provider, model).await.unwrap();
    let sid = sess.get_session_id(&config.provider);

    let mut send_req = bot.send_message(msg.chat.id, "⏳ [우덕터] Processing...");
    if let Some(tid) = msg.thread_id {
        send_req = send_req.message_thread_id(tid);
    }
    let proc_msg = match send_req.await {
        Ok(m) => m,
        Err(_) => return Ok(()),
    };

    let prompt = build_reply_prompt(msg, text);
    let opt_sid = if sid.is_empty() { None } else { Some(&sid[..]) };
    let stream_res = cli.send_streaming(&prompt, opt_sid, false, config.working_dir.clone()).await;

    match stream_res {
        Ok(stream) => {
            consume_stream(bot, msg.chat.id, proc_msg.id, msg.thread_id, stream, sessions, sess, config).await?;
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
) -> Result<(), teloxide::RequestError> {
    let from_id = msg.from().map(|u| u.id.0 as i64).unwrap_or(0);
    let chat_id_raw = msg.chat.id.0;
    if let Some(target_chat_id) = msg.migrate_to_chat_id() {
        let _ = sessions.migrate_chat_id(chat_id_raw, target_chat_id.0).await;
        return Ok(());
    }
    if !config.allowed_user_ids.contains(&from_id) && !config.allowed_group_ids.contains(&chat_id_raw) {
        return Ok(());
    }

    if let Some(text) = msg.text() {
        process_text(&bot, &msg, text, &config, &sessions, &cli, &cron_manager).await?;
    }
    Ok(())
}

pub async fn run_bot(config: CliConfig) -> Result<(), String> {
    let token = std::env::var("TELEGRAM_TOKEN")
        .unwrap_or_else(|_| config.telegram_token.clone());
    if token.is_empty() {
        return Err("No Telegram token provided".to_string());
    }

    let bot = Bot::new(token);
    let config_arc = Arc::new(config);
    
    let home = std::env::var("HOME").unwrap_or_else(|_| "/home/wimvm".to_string());
    let sessions_path = std::path::PathBuf::from(&home).join(".ductor").join("sessions.json");
    let sessions = Arc::new(crate::session::manager::SessionManager::new(
        sessions_path,
        30,
        4,
        false,
        "UTC".to_string(),
        None,
    ));
    
    let cli = Arc::new(AntigravityCli::new((*config_arc).clone()));

    let scheduler = Arc::new(crate::heartbeat::scheduler::HeartbeatScheduler::new(
        config_arc.clone(),
        sessions.clone(),
        cli.clone(),
    ));
    scheduler.start(bot.clone());

    let cron_path = std::path::PathBuf::from(&home).join(".ductor").join("cron_jobs.json");
    let cron_manager = Arc::new(crate::cron::manager::CronManager::new(cron_path));
    let cron_scheduler = Arc::new(crate::cron::scheduler::CronScheduler::new(
        config_arc.clone(),
        cron_manager.clone(),
        cli.clone(),
    ));
    cron_scheduler.start(bot.clone());

    let handler = dptree::entry()
        .branch(Update::filter_message().endpoint(handle_message))
        .branch(Update::filter_callback_query().endpoint(handle_callback_query));

    let mut dispatcher = Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![config_arc, sessions, cli, cron_manager])
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
