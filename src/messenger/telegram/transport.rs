//! # Telegram Message Transport Adapter
//!
//! ## Overview
//! Bridges standard background task output envelopes to Teloxide API calls. Supports text splitting,
//! quiet hour queuing, and forum topic routing.
//!
//! ## Collaboration Graph
//! - Registers as an observer on the central [`MessageBus`](crate::bus::bus::MessageBus).
//! - Feeds output strings through [`splitting::split_html_message`](super::formatting::splitting::split_html_message).
//!
//! ## Search Tags
//! #transport-adapter, #message-bus, #quiet-hour-queue, #topic-routing

use teloxide::prelude::*;
use crate::bus::bus::Transport;
use crate::bus::envelope::Envelope;
use crate::cli::AgentProvider;

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

pub struct TelegramPromptInjector {
    cli: std::sync::Arc<crate::cli::antigravity::AntigravityCli>,
    sess: std::sync::Arc<crate::session::manager::SessionManager>,
    cfg: std::sync::Arc<crate::config::CliConfig>,
}

impl TelegramPromptInjector {
    pub fn new(
        cli: std::sync::Arc<crate::cli::antigravity::AntigravityCli>,
        sess: std::sync::Arc<crate::session::manager::SessionManager>,
        cfg: std::sync::Arc<crate::config::CliConfig>,
    ) -> Self {
        Self { cli, sess, cfg }
    }
}

#[async_trait::async_trait]
impl crate::bus::bus::PromptInjector for TelegramPromptInjector {
    async fn inject_prompt(
        &self,
        prompt: &str,
        chat_id: i64,
        _label: &str,
        topic_id: Option<i64>,
        transport: &str,
    ) -> Result<String, String> {
        let key = crate::session::key::SessionKey::for_transport(transport, chat_id, topic_id);
        let model = self.cfg.model.as_deref().unwrap_or("gemini-3.8-flash");
        let (session, _) = self.sess.resolve_session(&key, &self.cfg.provider, model).await?;
        let sid = session.get_session_id(&self.cfg.provider);
        let opt_sid = if sid.is_empty() { None } else { Some(&sid[..]) };
        let resp = self.cli.send(prompt, opt_sid, false, self.cfg.working_dir.clone()).await?;
        Ok(resp.result)
    }
}
