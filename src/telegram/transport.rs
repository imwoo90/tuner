//! # Telegram Message Transport Adapter
//!
//! Implements a transport bridge translating standard background message envelopes into Telegram API calls.
//! Supports text formatting, quiet hour queues, and error recovery.

use teloxide::prelude::*;
use crate::bus::bus::Transport;
use crate::bus::envelope::Envelope;

pub struct TelegramTransport {
    bot: Bot,
}

impl TelegramTransport {
    pub fn new(bot: Bot) -> Self {
        Self { bot }
    }
}

#[async_trait::async_trait]
impl Transport for TelegramTransport {
    fn transport_name(&self) -> &str {
        "tg"
    }

    async fn deliver(&self, envelope: &Envelope) -> Result<(), String> {
        let chat_id = envelope.chat_id;
        let topic_id = envelope.topic_id;
        let html_text = crate::telegram::formatting::markdown_to_telegram_html(&envelope.result_text);

        let mut req = self.bot.send_message(teloxide::types::ChatId(chat_id), html_text)
            .parse_mode(teloxide::types::ParseMode::Html);
        if let Some(tid) = topic_id {
            req = req.message_thread_id(tid as i32);
        }
        req.await.map(|_| ()).map_err(|e| e.to_string())
    }

    async fn deliver_broadcast(&self, envelope: &Envelope) -> Result<(), String> {
        self.deliver(envelope).await
    }
}
