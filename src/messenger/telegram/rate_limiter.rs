//! # Telegram Chat-Wide Rate Limiter and Throttle
//!
//! ## Overview
//! Enforces global per-chat transmission rate limiting to prevent Telegram 429 RetryAfter
//! errors when multiple forum topics or concurrent sessions send messages or edit streaming text.
//!
//! ## Search Tags
//! #rate-limiter, #telegram-throttle, #retry-after

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use teloxide::prelude::*;
use teloxide::types::{MessageId, ParseMode, ThreadId};
use tokio::sync::Mutex;

pub(crate) fn is_message_not_modified(err: &teloxide::RequestError) -> bool {
    match err {
        teloxide::RequestError::Api(teloxide::ApiError::MessageNotModified) => true,
        other => other.to_string().to_lowercase().contains("message is not modified"),
    }
}

pub(crate) fn retry_backoff_secs(err: &teloxide::RequestError, attempts: usize) -> Option<u64> {
    if attempts >= 2 {
        return None;
    }
    match err {
        teloxide::RequestError::RetryAfter(sec) => Some(sec.seconds().max(1) as u64),
        teloxide::RequestError::Network(_) => Some((attempts as u64 + 1) * 2),
        _ => None,
    }
}

pub(crate) async fn send_single_chunk(
    bot: &Bot,
    chat_id: ChatId,
    msg_id: Option<MessageId>,
    thread_id: Option<i32>,
    chunk: &str,
    is_first: bool,
) -> Result<MessageId, teloxide::RequestError> {
    let limiter = global_chat_rate_limiter();
    let chunk_str = chunk.to_string();
    limiter.execute_rate_limited(chat_id, || {
        let c = chunk_str.clone();
        async move {
            let html = ParseMode::Html;
            if let (true, Some(mid)) = (is_first, msg_id) {
                match bot.edit_message_text(chat_id, mid, c.clone()).parse_mode(html).await {
                    Ok(ed) => return Ok(ed.id),
                    Err(e) if is_message_not_modified(&e) => return Ok(mid),
                    Err(e) if retry_backoff_secs(&e, 0).is_some() => return Err(e),
                    Err(e) => if e.to_string().contains("can't parse entities") {
                        if let Ok(ed) = bot.edit_message_text(chat_id, mid, c.clone()).await {
                            return Ok(ed.id);
                        }
                    } else {
                        eprintln!("⚠️ [tuner] edit_message_text fallback: {:?}", e);
                    },
                }
            }
            let tid = thread_id.map(|t| ThreadId(MessageId(t)));
            let mut req = bot.send_message(chat_id, c.clone()).parse_mode(html);
            if let Some(t) = tid {
                req = req.message_thread_id(t);
            }
            match req.await {
                Ok(m) => Ok(m.id),
                Err(e) if e.to_string().contains("can't parse entities") => {
                    let mut r = bot.send_message(chat_id, c);
                    if let Some(t) = tid {
                        r = r.message_thread_id(t);
                    }
                    r.await.map(|m| m.id)
                }
                Err(e) => Err(e),
            }
        }
    }).await
}

pub(crate) async fn send_chunks_to_telegram(
    bot: &Bot,
    chat_id: ChatId,
    msg_id: Option<MessageId>,
    thread_id: Option<i32>,
    chunks: &[String],
) -> (bool, Option<String>, Option<i32>) {
    let (mut cur_id, mut ok, mut err, mut sent_id) = (msg_id, true, None, None);
    for (i, chunk) in chunks.iter().enumerate() {
        match send_single_chunk(bot, chat_id, cur_id, thread_id, chunk, i == 0).await {
            Ok(mid) => {
                if cur_id.is_none() && i == 0 {
                    cur_id = Some(mid);
                }
                sent_id = Some(mid.0);
            }
            Err(e) => {
                eprintln!("❌ [tuner] Stream chunk {}/{} failed: {:?}", i + 1, chunks.len(), e);
                ok = false;
                err = Some(e.to_string());
            }
        }
    }
    (ok, err, sent_id)
}

/// Manages per-chat transmission intervals to avoid Telegram API rate limits.
pub struct ChatRateLimiter {
    chat_locks: Mutex<HashMap<i64, Arc<Mutex<Instant>>>>,
    min_interval: Duration,
}

static INSTANCE: OnceLock<Arc<ChatRateLimiter>> = OnceLock::new();

/// Returns the global singleton instance of [`ChatRateLimiter`].
pub fn global_chat_rate_limiter() -> Arc<ChatRateLimiter> {
    INSTANCE
        .get_or_init(|| Arc::new(ChatRateLimiter::new(Duration::from_millis(1000))))
        .clone()
}

impl ChatRateLimiter {
    /// Creates a new rate limiter with the specified minimum interval between operations per chat.
    pub fn new(min_interval: Duration) -> Self {
        Self {
            chat_locks: Mutex::new(HashMap::new()),
            min_interval,
        }
    }

    /// Obtains the per-chat mutex, ensuring calls to the same chat are synchronized.
    async fn get_chat_lock(&self, chat_id: ChatId) -> Arc<Mutex<Instant>> {
        let mut map = self.chat_locks.lock().await;
        map.entry(chat_id.0)
            .or_insert_with(|| Arc::new(Mutex::new(Instant::now() - self.min_interval)))
            .clone()
    }

    /// Executes an async Telegram operation ensuring at least `min_interval` has elapsed
    /// since the last operation in the same chat.
    pub async fn execute_rate_limited<F, Fut, T>(
        &self,
        chat_id: ChatId,
        mut action: F,
    ) -> Result<T, teloxide::RequestError>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = Result<T, teloxide::RequestError>>,
    {
        let chat_lock = self.get_chat_lock(chat_id).await;
        let mut last_sent = chat_lock.lock().await;
        let mut attempts = 0;

        loop {
            let elapsed = last_sent.elapsed();
            if elapsed < self.min_interval {
                tokio::time::sleep(self.min_interval - elapsed).await;
            }

            let res = action().await;
            *last_sent = Instant::now();

            match res {
                Err(ref err) if retry_backoff_secs(err, attempts).is_some() => {
                    let wait_secs = retry_backoff_secs(err, attempts).unwrap_or(2);
                    attempts += 1;
                    eprintln!(
                        "⚠️ [tuner] Telegram transient error ({}) for chat {}, backing off {}s (attempt {}/2)...",
                        err, chat_id, wait_secs, attempts
                    );
                    tokio::time::sleep(Duration::from_secs(wait_secs)).await;
                    *last_sent = Instant::now();
                }
                other => return other,
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn test_rate_limiter_spacing_same_chat() {
        let limiter = ChatRateLimiter::new(Duration::from_millis(50));
        let chat = ChatId(12345);

        let start = Instant::now();
        let _ = limiter.execute_rate_limited(chat, || async { Ok::<(), teloxide::RequestError>(()) }).await;
        let _ = limiter.execute_rate_limited(chat, || async { Ok::<(), teloxide::RequestError>(()) }).await;
        let elapsed = start.elapsed();

        assert!(elapsed >= Duration::from_millis(45), "Expected at least ~50ms spacing, got {:?}", elapsed);
    }

    #[tokio::test]
    async fn test_rate_limiter_independent_chats() {
        let limiter = ChatRateLimiter::new(Duration::from_millis(100));
        let chat1 = ChatId(111);
        let chat2 = ChatId(222);

        let start = Instant::now();
        let _ = limiter.execute_rate_limited(chat1, || async { Ok::<(), teloxide::RequestError>(()) }).await;
        let _ = limiter.execute_rate_limited(chat2, || async { Ok::<(), teloxide::RequestError>(()) }).await;
        let elapsed = start.elapsed();

        assert!(elapsed < Duration::from_millis(90), "Different chats should run concurrently without waiting, got {:?}", elapsed);
    }

    #[tokio::test]
    async fn test_rate_limiter_retries_transient_error() {
        let limiter = ChatRateLimiter::new(Duration::from_millis(10));
        let chat = ChatId(333);
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_ref = calls.clone();

        let res = limiter.execute_rate_limited(chat, || {
            let c = calls_ref.clone();
            async move {
                let cur = c.fetch_add(1, Ordering::SeqCst);
                if cur == 0 {
                    Err(teloxide::RequestError::RetryAfter(teloxide::types::Seconds::from_seconds(1)))
                } else {
                    Ok("recovered")
                }
            }
        }).await;

        assert_eq!(res.unwrap(), "recovered");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(retry_backoff_secs(&teloxide::RequestError::Api(teloxide::ApiError::MessageNotModified), 0), None);
    }
}
