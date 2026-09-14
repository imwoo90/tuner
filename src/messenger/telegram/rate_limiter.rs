//! # Telegram Chat-Wide Rate Limiter and Throttle
//!
//! Enforces global per-chat transmission rate limiting to prevent Telegram 429 RetryAfter
//! errors when multiple forum topics or concurrent sessions send messages or edit streaming text.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use teloxide::prelude::*;
use tokio::sync::Mutex;

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
                Err(teloxide::RequestError::RetryAfter(sec)) if attempts < 2 => {
                    attempts += 1;
                    let wait_secs = sec.seconds().max(1) as u64;
                    eprintln!(
                        "⚠️ [tuner] Rate limit RetryAfter({}s) encountered for chat {}, backing off (attempt {}/2)...",
                        wait_secs, chat_id, attempts
                    );
                    tokio::time::sleep(Duration::from_secs(wait_secs)).await;
                    *last_sent = Instant::now();
                    continue;
                }
                other => return other,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
