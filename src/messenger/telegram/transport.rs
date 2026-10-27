//! # Telegram Message Transport Adapter
//!
//! ## Overview
//! Bridges standard background task output envelopes to Teloxide API calls. Supports text splitting,
//! quiet hour queuing, and forum topic routing.
//!
//! ## Search Tags
//! #transport-adapter, #message-bus, #quiet-hour-queue, #topic-routing

use std::sync::Arc;
use teloxide::prelude::*;
use crate::bus::bus::Transport;
use crate::bus::envelope::Envelope;
use crate::config::CliConfig;
use crate::session::manager::SessionManager;

/// Represents `TelegramTransport` (telegram transport).
pub struct TelegramTransport {
    bot: Bot,
    broadcast_targets: Vec<i64>,
    config: Option<Arc<CliConfig>>,
    sessions: Option<Arc<SessionManager>>,
}

impl TelegramTransport {
    /// Creates a new instance with configured broadcast chat/group targets.
    pub fn with_targets(bot: Bot, broadcast_targets: Vec<i64>) -> Self {
        Self {
            bot,
            broadcast_targets,
            config: None,
            sessions: None,
        }
    }

    /// Attaches workspace configuration and session manager for file review links and topic history logging.
    pub fn with_context(mut self, config: Arc<CliConfig>, sessions: Arc<SessionManager>) -> Self {
        self.config = Some(config);
        self.sessions = Some(sessions);
        self
    }

    async fn log_to_topic_history(
        &self,
        cfg: &CliConfig,
        sessions: &SessionManager,
        envelope: &Envelope,
        sent_id: Option<i32>,
        status: (bool, Option<&str>),
    ) {
        let key = crate::session::key::SessionKey::telegram(envelope.chat_id, envelope.topic_id);
        let Ok(Some(sess)) = sessions.get_active(&key).await else {
            return;
        };
        let active_sid = sess.get_session_id(&cfg.provider);
        let target_sid = if !active_sid.is_empty() {
            active_sid
        } else {
            sess.get_past_session_ids(&cfg.provider).last().cloned().unwrap_or_default()
        };
        if !target_sid.is_empty() {
            super::history::log_telegram_message(
                &cfg.working_dir,
                &target_sid,
                (envelope.topic_id, sess.topic_name.as_deref()),
                "bot",
                sent_id,
                &envelope.result_text,
                status,
            );
        }
    }
}

#[async_trait::async_trait]
impl Transport for TelegramTransport {
    fn transport_name(&self) -> &str {
        "tg"
    }

    async fn deliver(&self, envelope: &Envelope) -> Result<(), String> {
        if envelope.chat_id == 0 {
            return Err("Cannot deliver unicast to chat_id 0".to_string());
        }
        let chat_id = teloxide::types::ChatId(envelope.chat_id);
        let thread_id = envelope.topic_id.map(|t| t as i32);
        let html_text = crate::telegram::formatting::markdown_to_telegram_html(&envelope.result_text);
        let chunks = crate::telegram::formatting::splitting::split_html_message(&html_text, 4000);
        let (ok, err, sent_id) = crate::telegram::stream::send_chunks_to_telegram(&self.bot, chat_id, None, thread_id, &chunks).await;
        if let Some(ref cfg) = self.config {
            super::attachments::send_file_attachments(&self.bot, chat_id, thread_id, &envelope.result_text, cfg).await;
            if let Some(ref sessions) = self.sessions {
                let status = if envelope.is_error {
                    (false, Some(envelope.status.as_str()))
                } else if cfg!(test) {
                    (true, None)
                } else {
                    (ok, err.as_deref())
                };
                self.log_to_topic_history(cfg, sessions, envelope, sent_id, status).await;
            }
        }
        Ok(())
    }

    async fn deliver_broadcast(&self, envelope: &Envelope) -> Result<(), String> {
        if envelope.chat_id != 0 {
            return self.deliver(envelope).await;
        }
        for &target_id in &self.broadcast_targets {
            if target_id != 0 {
                let mut target_env = envelope.clone();
                target_env.chat_id = target_id;
                let _ = self.deliver(&target_env).await;
            }
        }
        Ok(())
    }
}
