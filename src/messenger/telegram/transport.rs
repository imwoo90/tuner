//! # Telegram Message Transport Adapter
//!
//! ## Overview
//! Bridges standard background task output envelopes to Teloxide API calls. Supports text splitting,
//! quiet hour queuing, and forum topic routing.
//!
//! ## Search Tags
//! #transport-adapter, #message-bus, #quiet-hour-queue, #topic-routing

use teloxide::prelude::*;
use crate::bus::bus::Transport;
use crate::bus::envelope::Envelope;

/// Represents `TelegramTransport` (telegram transport).
pub struct TelegramTransport {
    bot: Bot,
    broadcast_targets: Vec<i64>,
}

impl TelegramTransport {
    /// Creates a new instance with configured broadcast chat/group targets.
    pub fn with_targets(bot: Bot, broadcast_targets: Vec<i64>) -> Self {
        Self {
            bot,
            broadcast_targets,
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
        crate::telegram::stream::send_chunks_to_telegram(&self.bot, chat_id, None, thread_id, &chunks).await;
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
