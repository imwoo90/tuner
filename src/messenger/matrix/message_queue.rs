//! Message queue for Matrix bot: dedup, pending task tracking, and drain.
//!
//! Brings the Matrix transport closer to parity with Telegram's
//! ConversationMiddleware by preventing duplicate processing, tracking
//! in-flight message tasks, and cancelling queued work on /stop.

//! 
//! ## Search Tags
//! #message-queue

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Mutex;
use tokio::task::JoinHandle;

#[derive(Debug)]
pub struct DedupeCache {
    seen: Mutex<HashSet<String>>,
    queue: Mutex<VecDeque<String>>,
    capacity: usize,
}

impl DedupeCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            seen: Mutex::new(HashSet::new()),
            queue: Mutex::new(VecDeque::new()),
            capacity,
        }
    }

    pub fn check(&self, event_id: &str) -> bool {
        let mut seen = self.seen.lock().unwrap();
        if seen.contains(event_id) {
            return true;
        }

        let mut queue = self.queue.lock().unwrap();
        seen.insert(event_id.to_string());
        queue.push_back(event_id.to_string());
        if queue.len() > self.capacity {
            if let Some(oldest) = queue.pop_front() {
                seen.remove(&oldest);
            }
        }
        false
    }
}

#[derive(Debug)]
pub struct MatrixMessageQueue {
    dedup: DedupeCache,
    pending: Mutex<HashMap<i64, Vec<JoinHandle<()>>>>,
}

impl MatrixMessageQueue {
    pub fn new() -> Self {
        Self {
            dedup: DedupeCache::new(1000),
            pending: Mutex::new(HashMap::new()),
        }
    }

    pub fn is_duplicate(&self, event_id: &str) -> bool {
        self.dedup.check(event_id)
    }

    pub fn track(&self, chat_id: i64, task: JoinHandle<()>) {
        let mut pending = self.pending.lock().unwrap();
        pending.entry(chat_id).or_default().push(task);
    }

    pub fn pending_count(&self, chat_id: i64) -> usize {
        let mut pending = self.pending.lock().unwrap();
        if let Some(tasks) = pending.get_mut(&chat_id) {
            tasks.retain(|t| !t.is_finished());
            tasks.len()
        } else {
            0
        }
    }

    pub fn is_busy(&self, chat_id: i64) -> bool {
        self.pending_count(chat_id) > 0
    }

    pub fn drain(&self, chat_id: i64) -> usize {
        let mut pending = self.pending.lock().unwrap();
        let mut cancelled = 0;
        if let Some(tasks) = pending.remove(&chat_id) {
            for task in tasks {
                if !task.is_finished() {
                    task.abort();
                    cancelled += 1;
                }
            }
        }
        cancelled
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_first_event_is_not_duplicate() {
        let q = MatrixMessageQueue::new();
        assert!(!q.is_duplicate("$event1"));
    }

    #[test]
    fn test_same_event_id_is_duplicate() {
        let q = MatrixMessageQueue::new();
        q.is_duplicate("$event1");
        assert!(q.is_duplicate("$event1"));
    }

    #[test]
    fn test_different_event_ids_are_not_duplicates() {
        let q = MatrixMessageQueue::new();
        q.is_duplicate("$event1");
        assert!(!q.is_duplicate("$event2"));
    }

    #[test]
    fn test_no_pending_initially() {
        let q = MatrixMessageQueue::new();
        assert_eq!(q.pending_count(1), 0);
    }

    #[tokio::test]
    async fn test_track_adds_task() {
        let q = MatrixMessageQueue::new();
        let task = tokio::spawn(async {
            tokio::time::sleep(Duration::from_secs(10)).await;
        });
        q.track(1, task);
        assert_eq!(q.pending_count(1), 1);
        q.drain(1);
    }

    #[tokio::test]
    async fn test_track_multiple_tasks() {
        let q = MatrixMessageQueue::new();
        let t1 = tokio::spawn(async { tokio::time::sleep(Duration::from_secs(10)).await; });
        let t2 = tokio::spawn(async { tokio::time::sleep(Duration::from_secs(10)).await; });
        q.track(1, t1);
        q.track(1, t2);
        assert_eq!(q.pending_count(1), 2);
        q.drain(1);
    }

    #[tokio::test]
    async fn test_tasks_for_different_chats_are_separate() {
        let q = MatrixMessageQueue::new();
        let t1 = tokio::spawn(async { tokio::time::sleep(Duration::from_secs(10)).await; });
        let t2 = tokio::spawn(async { tokio::time::sleep(Duration::from_secs(10)).await; });
        q.track(1, t1);
        q.track(2, t2);
        assert_eq!(q.pending_count(1), 1);
        assert_eq!(q.pending_count(2), 1);
        q.drain(1);
        q.drain(2);
    }

    #[tokio::test]
    async fn test_completed_tasks_are_pruned() {
        let q = MatrixMessageQueue::new();
        let t = tokio::spawn(async {});
        q.track(1, t);
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(q.pending_count(1), 0);
    }

    #[tokio::test]
    async fn test_drain_cancels_pending_tasks() {
        let q = MatrixMessageQueue::new();
        let t1 = tokio::spawn(async { tokio::time::sleep(Duration::from_secs(10)).await; });
        let t2 = tokio::spawn(async { tokio::time::sleep(Duration::from_secs(10)).await; });
        q.track(1, t1);
        q.track(1, t2);
        let count = q.drain(1);
        assert_eq!(count, 2);
    }

    #[test]
    fn test_drain_returns_zero_when_no_pending() {
        let q = MatrixMessageQueue::new();
        assert_eq!(q.drain(1), 0);
    }

    #[tokio::test]
    async fn test_drain_does_not_affect_other_chats() {
        let q = MatrixMessageQueue::new();
        let t1 = tokio::spawn(async { tokio::time::sleep(Duration::from_secs(10)).await; });
        let t2 = tokio::spawn(async { tokio::time::sleep(Duration::from_secs(10)).await; });
        q.track(1, t1);
        q.track(2, t2);
        q.drain(1);
        assert_eq!(q.pending_count(2), 1);
        q.drain(2);
    }

    #[tokio::test]
    async fn test_drain_skips_already_done_tasks() {
        let q = MatrixMessageQueue::new();
        let t1 = tokio::spawn(async {});
        tokio::time::sleep(Duration::from_millis(50)).await;
        let t2 = tokio::spawn(async { tokio::time::sleep(Duration::from_secs(10)).await; });
        q.track(1, t1);
        q.track(1, t2);
        let count = q.drain(1);
        assert_eq!(count, 1);
    }

    #[test]
    fn test_not_busy_when_empty() {
        let q = MatrixMessageQueue::new();
        assert!(!q.is_busy(1));
    }

    #[tokio::test]
    async fn test_busy_when_has_pending() {
        let q = MatrixMessageQueue::new();
        let t = tokio::spawn(async { tokio::time::sleep(Duration::from_secs(10)).await; });
        q.track(1, t);
        assert!(q.is_busy(1));
        q.drain(1);
    }
}
