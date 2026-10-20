//! # Media Group Debouncer and Attachment Aggregator
//!
//! ## Overview
//! Collects clustered media files sent in rapid succession via Telegram's media groups,
//! debouncing them into single cohesive events with multiple files.
//!
//! ## Search Tags
//! #media-group, #debouncing, #attachment-aggregator

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

#[derive(Clone)]
/// Dependency bundle for media group processing and message routing.
pub struct MediaGroupDeps {
    /// Active CLI configuration.
    pub config: Arc<CliConfig>,
    /// Active session manager.
    pub sessions: Arc<SessionManager>,
    /// Antigravity CLI controller.
    pub cli: Arc<AntigravityCli>,
    /// Cron job manager.
    pub cron_manager: Arc<CronManager>,
    /// Forum topic name cache.
    pub topic_cache: Arc<TopicNameCache>,
    /// Bot identity metadata.
    pub bot_info: Arc<BotInfo>,
}

/// Represents `MediaGroupManager` (media group manager).
pub struct MediaGroupManager {
    buffers: Arc<Mutex<HashMap<String, MediaGroupBuffer>>>,
}

struct MediaGroupBuffer {
    messages: Vec<Message>,
    cancel_tx: mpsc::Sender<()>,
}

impl Default for MediaGroupManager {
    fn default() -> Self {
        Self::new()
    }
}

impl MediaGroupManager {
    /// Creates a new instance.
    pub fn new() -> Self {
        Self {
            buffers: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Executes `add_message` (add message).
    pub async fn add_message(
        &self,
        bot: Bot,
        msg: Message,
        group_id: String,
        deps: MediaGroupDeps,
    ) {
        let buffers = self.buffers.clone();
        let mut tx_to_notify = None;

        {
            let mut guard = buffers.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(buf) = guard.get_mut(&group_id) {
                buf.messages.push(msg.clone());
                tx_to_notify = Some(buf.cancel_tx.clone());
            } else {
                let (tx, rx) = mpsc::channel(10);
                guard.insert(
                    group_id.clone(),
                    MediaGroupBuffer {
                        messages: vec![msg.clone()],
                        cancel_tx: tx,
                    },
                );
                spawn_timer_task(group_id, buffers.clone(), rx, bot, deps);
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
    deps: MediaGroupDeps,
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
                        let mut guard = buffers.lock().unwrap_or_else(|e| e.into_inner());
                        guard.remove(&group_id).map(|b| b.messages).unwrap_or_default()
                    };
                    if !messages.is_empty()
                        && let Err(e) = process_media_group(bot.clone(), messages, deps.clone()).await {
                            eprintln!("❌ [tuner] process_media_group failed: {:?}", e);
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
    deps: MediaGroupDeps,
) -> Result<(), teloxide::RequestError> {
    let mut downloaded_files = Vec::new();
    let dest_dir = deps.config.working_dir.join("telegram_files");
    
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
            (&caption, &downloaded_files),
            &deps.config,
            &deps.sessions,
            &deps.cli,
            (&deps.cron_manager, &deps.topic_cache),
        ).await?;
    }

    Ok(())
}
