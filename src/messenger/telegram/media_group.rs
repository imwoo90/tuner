//! # Media Group Debouncer and Attachment Aggregator
//!
//! ## Overview
//! Collects clustered media files sent in rapid succession via Telegram's media groups,
//! debouncing them into single cohesive events with multiple files.
//!
//! ## Search Tags
//! #media-group, #debouncing, #attachment-aggregator

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use teloxide::prelude::*;
use tokio::sync::mpsc;
use crate::config::CliConfig;
use crate::session::manager::SessionManager;
use crate::cli::antigravity::AntigravityCli;
use crate::cron::manager::CronManager;
use crate::telegram::topic_cache::TopicNameCache;
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
    /// Bot username for stripping mentions in group captions.
    pub bot_username: Option<String>,
}

/// Represents `MediaGroupManager` (media group manager).
pub struct MediaGroupManager {
    buffers: Arc<Mutex<HashMap<String, MediaGroupBuffer>>>,
    next_seq: AtomicU64,
}

type TargetTopicResult = Result<Option<i64>, String>;
type DrainedMediaGroup = (Vec<Message>, Option<i64>, TargetTopicResult);
type GroupLockContext = (i64, Option<i64>, TargetTopicResult);

struct MediaGroupBuffer {
    seq: u64,
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
            next_seq: AtomicU64::new(0),
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
                buf.messages.push(msg);
                tx_to_notify = Some(buf.cancel_tx.clone());
            } else {
                let (tx, rx) = mpsc::channel(10);
                let seq = self.next_seq.fetch_add(1, Ordering::Relaxed);
                guard.insert(
                    group_id.clone(),
                    MediaGroupBuffer {
                        seq,
                        messages: vec![msg],
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

    /// Synchronously flushes any buffered media groups targeting `(chat_id, held_lock_tid)`
    /// (excluding `exclude_group_id`) while the caller already holds that topic's `lock_pool` mutex.
    pub async fn flush_other_groups_for_topic(
        &self,
        bot: &Bot,
        chat_id: i64,
        held_lock_tid: Option<i64>,
        exclude_group_id: Option<&str>,
        deps: &MediaGroupDeps,
    ) -> Result<(), teloxide::RequestError> {
        let ready = self.drain_matching_groups(chat_id, held_lock_tid, exclude_group_id, deps);
        for (messages, msg_tid, target_res) in ready {
            let active_lang = super::handler::resolve_active_lang(&deps.sessions, &deps.config, chat_id, msg_tid).await;
            let fut = super::handler::LOCKED_TOPIC_ID.scope(
                target_res,
                process_media_group(bot.clone(), messages, deps.clone()),
            );
            crate::i18n::TASK_ACTIVE_LANG.scope(active_lang, fut).await?;
            crate::upgrade::record_user_activity();
        }
        Ok(())
    }

    fn drain_matching_groups(
        &self,
        chat_id: i64,
        held_lock_tid: Option<i64>,
        exclude_group_id: Option<&str>,
        deps: &MediaGroupDeps,
    ) -> Vec<DrainedMediaGroup> {
        let mut guard = self.buffers.lock().unwrap_or_else(|e| e.into_inner());
        let mut matching: Vec<(u64, String)> = guard
            .iter()
            .filter_map(|(gid, buf)| {
                if exclude_group_id == Some(gid.as_str()) {
                    return None;
                }
                let (cid, msg_tid, target_res) = resolve_group_lock_context(&buf.messages, deps)?;
                let lock_tid = *target_res.as_ref().unwrap_or(&msg_tid);
                ((cid, lock_tid) == (chat_id, held_lock_tid)).then(|| (buf.seq, gid.clone()))
            })
            .collect();
        matching.sort_by_key(|(seq, _)| *seq);
        let mut drained = Vec::with_capacity(matching.len());
        for (_, gid) in matching {
            if let Some(buf) = guard.remove(&gid)
                && let Some((_, msg_tid, target_res)) = resolve_group_lock_context(&buf.messages, deps)
            {
                drained.push((buf.messages, msg_tid, target_res));
            }
        }
        drained
    }
}

fn resolve_group_lock_context(
    messages: &[Message],
    deps: &MediaGroupDeps,
) -> Option<GroupLockContext> {
    let mut sorted: Vec<&Message> = messages.iter().collect();
    sorted.sort_by_key(|m| m.id.0);
    let mut caption = String::new();
    for msg in &sorted {
        let txt = msg.text().or(msg.caption()).unwrap_or("").trim();
        if !txt.is_empty() && caption.is_empty() {
            caption = super::handler::normalize_command_text(txt, deps.bot_username.as_deref());
        }
    }
    let trigger = sorted
        .iter()
        .copied()
        .find(|m| m.reply_to_message().is_some())
        .or_else(|| sorted.iter().copied().find(|m| !m.text().or(m.caption()).unwrap_or("").trim().is_empty()))
        .or_else(|| sorted.first().copied())?;
    let msg_tid = super::get_topic_id(trigger);
    let target_res = super::handler::resolve_command_target_topic(trigger, &caption, &deps.topic_cache, msg_tid);
    Some((trigger.chat.id.0, msg_tid, target_res))
}

async fn flush_group_on_timeout(
    group_id: &str,
    buffers: &Arc<Mutex<HashMap<String, MediaGroupBuffer>>>,
    bot: &Bot,
    deps: &MediaGroupDeps,
) {
    loop {
        let peeked = {
            let guard = buffers.lock().unwrap_or_else(|e| e.into_inner());
            guard.get(group_id).and_then(|b| {
                let (cid, msg_tid, target_res) = resolve_group_lock_context(&b.messages, deps)?;
                Some((cid, *target_res.as_ref().unwrap_or(&msg_tid)))
            })
        };
        let Some((chat_id, want_lock_tid)) = peeked else { return; };
        let topic_lock = deps.sessions.lock_pool.get((chat_id, want_lock_tid));
        let _topic_guard = topic_lock.lock().await;
        let taken = {
            let mut guard = buffers.lock().unwrap_or_else(|e| e.into_inner());
            let Some(cur_ctx) = guard.get(group_id).and_then(|b| resolve_group_lock_context(&b.messages, deps)) else {
                return;
            };
            let cur_lock_tid = *cur_ctx.2.as_ref().unwrap_or(&cur_ctx.1);
            if (cur_ctx.0, cur_lock_tid) != (chat_id, want_lock_tid) {
                None
            } else {
                guard.remove(group_id).map(|b| (b.messages, cur_ctx.1, cur_ctx.2))
            }
        };
        let Some((messages, msg_tid, target_res)) = taken else { continue; };
        let active_lang = super::handler::resolve_active_lang(&deps.sessions, &deps.config, chat_id, msg_tid).await;
        let fut = super::handler::LOCKED_TOPIC_ID.scope(
            target_res,
            process_media_group(bot.clone(), messages, deps.clone()),
        );
        if let Err(e) = crate::i18n::TASK_ACTIVE_LANG.scope(active_lang, fut).await {
            eprintln!("❌ [tuner] process_media_group failed: {:?}", e);
        }
        crate::upgrade::record_user_activity();
        return;
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
                msg = rx.recv() => {
                    if msg.is_none() {
                        break;
                    }
                }
                _ = tokio::time::sleep(Duration::from_millis(1500)) => {
                    flush_group_on_timeout(&group_id, &buffers, &bot, &deps).await;
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

    // Find the caption text (first non-empty text/caption in the group) and normalize mentions/commands
    let mut caption = String::new();
    for msg in &sorted_messages {
        let txt = msg.text().or(msg.caption()).unwrap_or("").trim();
        if !txt.is_empty() && caption.is_empty() {
            caption = super::handler::normalize_command_text(txt, deps.bot_username.as_deref());
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

    // Prefer the message carrying a reply quote or caption as the triggering context
    let trigger_msg = sorted_messages
        .iter()
        .find(|m| m.reply_to_message().is_some())
        .or_else(|| sorted_messages.iter().find(|m| !m.text().or(m.caption()).unwrap_or("").trim().is_empty()))
        .or_else(|| sorted_messages.first());
    if let Some(trigger_msg) = trigger_msg {
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
