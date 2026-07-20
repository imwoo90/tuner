//! # Media Group Debouncer and Attachment Aggregator
//!
//! Collects clustered media files (images, files, documents) sent in rapid succession
//! via Telegram's media groups, debouncing them into single cohesive events with multiple files.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use teloxide::prelude::*;
use tokio::sync::mpsc;
use crate::config::CliConfig;
use crate::session::manager::SessionManager;
use crate::cli::antigravity::AntigravityCli;
use crate::cron::manager::CronManager;
use crate::telegram::topic_cache::{BotInfo, TopicNameCache};
use crate::telegram::reply::download_telegram_media;

pub struct MediaGroupManager {
    buffers: Arc<Mutex<HashMap<String, MediaGroupBuffer>>>,
}

struct MediaGroupBuffer {
    messages: Vec<Message>,
    cancel_tx: mpsc::Sender<()>,
}

impl MediaGroupManager {
    pub fn new() -> Self {
        Self {
            buffers: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn add_message(
        &self,
        bot: Bot,
        msg: Message,
        group_id: String,
        config: Arc<CliConfig>,
        sessions: Arc<SessionManager>,
        cli: Arc<AntigravityCli>,
        cron_manager: Arc<CronManager>,
        topic_cache: Arc<TopicNameCache>,
        bot_info: Arc<BotInfo>,
    ) {
        let buffers = self.buffers.clone();
        let mut tx_to_notify = None;

        {
            let mut guard = buffers.lock().unwrap();
            if let Some(buf) = guard.get_mut(&group_id) {
                buf.messages.push(msg.clone());
                tx_to_notify = Some(buf.cancel_tx.clone());
            } else {
                let (tx, rx) = mpsc::channel(10);
                guard.insert(group_id.clone(), MediaGroupBuffer {
                    messages: vec![msg.clone()],
                    cancel_tx: tx,
                });
                spawn_timer_task(
                    group_id,
                    buffers.clone(),
                    rx,
                    bot,
                    config,
                    sessions,
                    cli,
                    cron_manager,
                    topic_cache,
                    bot_info,
                );
            }
        }

        if let Some(tx) = tx_to_notify {
            let _ = tx.send(()).await;
        }
    }
}

fn spawn_timer_task(
    group_id: String,
    buffers: Arc<Mutex<HashMap<String, MediaGroupBuffer>>>,
    mut rx: mpsc::Receiver<()>,
    bot: Bot,
    config: Arc<CliConfig>,
    sessions: Arc<SessionManager>,
    cli: Arc<AntigravityCli>,
    cron_manager: Arc<CronManager>,
    topic_cache: Arc<TopicNameCache>,
    bot_info: Arc<BotInfo>,
) {
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = rx.recv() => {
                    // Extended, keep waiting
                }
                _ = tokio::time::sleep(Duration::from_millis(1500)) => {
                    // Timeout reached, flush group
                    let messages = {
                        let mut guard = buffers.lock().unwrap();
                        guard.remove(&group_id).map(|b| b.messages).unwrap_or_default()
                    };
                    if !messages.is_empty() {
                        if let Err(e) = process_media_group(
                            bot.clone(),
                            messages,
                            config.clone(),
                            sessions.clone(),
                            cli.clone(),
                            cron_manager.clone(),
                            topic_cache.clone(),
                            bot_info.clone(),
                        ).await {
                            eprintln!("❌ [tuner] process_media_group failed: {:?}", e);
                        }
                    }
                    break;
                }
            }
        }
    });
}

pub(crate) async fn process_media_group(
    bot: Bot,
    messages: Vec<Message>,
    config: Arc<CliConfig>,
    sessions: Arc<SessionManager>,
    cli: Arc<AntigravityCli>,
    cron_manager: Arc<CronManager>,
    topic_cache: Arc<TopicNameCache>,
    bot_info: Arc<BotInfo>,
) -> Result<(), teloxide::RequestError> {
    let mut downloaded_files = Vec::new();
    let dest_dir = config.working_dir.join("telegram_files");
    
    // Sort messages by ID to have deterministic order of files
    let mut sorted_messages = messages;
    sorted_messages.sort_by_key(|m| m.id.0);

    // Find the caption text (first non-empty text/caption in the group)
    let mut caption = String::new();
    for msg in &sorted_messages {
        let txt = msg.text().or(msg.caption()).unwrap_or("").trim().to_string();
        if !txt.is_empty() && caption.is_empty() {
            caption = txt;
        }
    }

    for msg in &sorted_messages {
        match download_telegram_media(&bot, msg, &dest_dir).await {
            Ok(Some(relative_path)) => {
                downloaded_files.push(relative_path);
            }
            Ok(None) => {}
            Err(e) => {
                eprintln!("Error downloading media in group: {:?}", e);
            }
        }
    }

    // Use the first message of the group as the main triggering message context
    if let Some(trigger_msg) = sorted_messages.first() {
        crate::telegram::process_text_with_files(
            &bot,
            trigger_msg,
            &caption,
            &downloaded_files,
            &config,
            &sessions,
            &cli,
            &cron_manager,
            &topic_cache,
        ).await?;
    }

    Ok(())
}
