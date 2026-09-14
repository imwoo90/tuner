//! # Telegram Typing Status Indicator Guard
//!
//! ## Overview
//! Periodically sends the "typing..." chat action to Telegram while an agent is generating code
//! or executing background tasks. Dropped automatically when execution ends.
//!
//! ## Collaboration Graph
//! - Spawned during PTY execution streams.
//! - Binds directly to the Teloxide client handle.
//!
//! ## Search Tags
//! #typing-indicator, #status-guard, #user-feedback

use std::time::Duration;
use tokio::task::JoinHandle;
use teloxide::prelude::*;
use teloxide::types::ChatAction;

/// RAII guard for Telegram typing activity and message reaction.
pub struct TelegramTypingGuard {
    token: String,
    chat_id: ChatId,
    message_id: teloxide::types::MessageId,
    handle: JoinHandle<()>,
}

impl TelegramTypingGuard {
    /// Create a new guard, setting the typing action and reactions.
    pub async fn new(
        bot: Bot,
        token: String,
        msg: &Message,
    ) -> Self {
        let chat_id = msg.chat.id;
        let message_id = msg.id;
        let thread_id = msg.thread_id;

        let bot_clone = bot.clone();
        tokio::spawn(async move {
            let mut req = bot_clone.send_chat_action(chat_id, ChatAction::Typing);
            if let Some(t) = thread_id {
                req = req.message_thread_id(t);
            }
            let _ = req.await;
        });

        let token_clone = token.clone();
        tokio::spawn(async move {
            let client = reqwest::Client::new();
            let url = format!("https://api.telegram.org/bot{}/setMessageReaction", token_clone);
            let body = serde_json::json!({
                "chat_id": chat_id.0,
                "message_id": message_id.0,
                "reaction": [
                    {
                        "type": "emoji",
                        "emoji": "👀"
                    }
                ]
            });
            let _ = client.post(&url).json(&body).send().await;
        });

        let bot_clone3 = bot.clone();
        let handle = tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(4)).await;
                let mut req = bot_clone3.send_chat_action(chat_id, ChatAction::Typing);
                if let Some(t) = thread_id {
                    req = req.message_thread_id(t);
                }
                let _ = req.await;
            }
        });

        Self {
            token,
            chat_id,
            message_id,
            handle,
        }
    }
}

impl Drop for TelegramTypingGuard {
    fn drop(&mut self) {
        self.handle.abort();
        let token = self.token.clone();
        let chat_id = self.chat_id;
        let message_id = self.message_id;
        tokio::spawn(async move {
            let client = reqwest::Client::new();
            let url = format!("https://api.telegram.org/bot{}/setMessageReaction", token);
            let body = serde_json::json!({
                "chat_id": chat_id.0,
                "message_id": message_id.0,
                "reaction": []
            });
            let _ = client.post(&url).json(&body).send().await;
        });
    }
}

pub(crate) fn clear_old_progress_reaction(last_mid: i32, chat_id: ChatId, tok: String) {
    tokio::spawn(async move {
        let client = reqwest::Client::new();
        let url = format!("https://api.telegram.org/bot{}/setMessageReaction", tok);
        let body = serde_json::json!({
            "chat_id": chat_id.0,
            "message_id": last_mid,
            "reaction": []
        });
        let _ = client.post(&url).json(&body).send().await;
    });
}

pub(crate) fn set_progress_reaction(msg_id_val: i32, chat_id: ChatId, tok: String) {
    tokio::spawn(async move {
        let client = reqwest::Client::new();
        let url = format!("https://api.telegram.org/bot{}/setMessageReaction", tok);
        let body = serde_json::json!({
            "chat_id": chat_id.0,
            "message_id": msg_id_val,
            "reaction": [
                {
                    "type": "emoji",
                    "emoji": "⏳"
                }
            ]
        });
        let _ = client.post(&url).json(&body).send().await;
    });
}

/// Background loop handle for periodic ChatAction::Typing during async turns.
pub struct AsyncTypingHandle {
    handle: JoinHandle<()>,
}

impl AsyncTypingHandle {
    pub fn new(bot: Bot, chat_id: ChatId, thread_id: Option<i32>) -> Self {
        let handle = tokio::spawn(async move {
            let mut req = bot.send_chat_action(chat_id, ChatAction::Typing);
            if let Some(t) = thread_id {
                req = req.message_thread_id(teloxide::types::ThreadId(teloxide::types::MessageId(t)));
            }
            if let Err(e) = req.await {
                eprintln!("⚠️ [tuner] Async typing send_chat_action failed (chat: {}, thread: {:?}): {:?}", chat_id, thread_id, e);
            }

            loop {
                tokio::time::sleep(Duration::from_secs(4)).await;
                let mut req = bot.send_chat_action(chat_id, ChatAction::Typing);
                if let Some(t) = thread_id {
                    req = req.message_thread_id(teloxide::types::ThreadId(teloxide::types::MessageId(t)));
                }
                if let Err(e) = req.await {
                    eprintln!("⚠️ [tuner] Async typing periodic send_chat_action failed (chat: {}, thread: {:?}): {:?}", chat_id, thread_id, e);
                }
            }
        });

        Self { handle }
    }

    pub fn abort(&self) {
        self.handle.abort();
    }
}

impl Drop for AsyncTypingHandle {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_telegram_typing_guard_lifecycle() {
        let bot = Bot::new("123456:ABC-DEF");
        let msg: Message = serde_json::from_str(
            r#"{"message_id":67890,"date":1,"chat":{"id":12345,"type":"private"},"from":{"id":100,"is_bot":false,"first_name":"I","username":"u"},"text":"test"}"#
        ).unwrap();

        let guard = TelegramTypingGuard::new(bot, "123456:ABC-DEF".to_string(), &msg).await;
        tokio::time::sleep(Duration::from_millis(50)).await;
        drop(guard);
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    #[tokio::test]
    async fn test_async_typing_handle_lifecycle() {
        let bot = Bot::new("123456:ABC-DEF");
        let handle = AsyncTypingHandle::new(bot, ChatId(12345), Some(999));
        tokio::time::sleep(Duration::from_millis(50)).await;
        handle.abort();
        drop(handle);
    }
}

