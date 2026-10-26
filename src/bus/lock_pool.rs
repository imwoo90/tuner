//! # Lock Pool Module
//!
//! ## Overview
//! Provides a lock management system (`LockPool`) that tracks active asynchronous locks
//! mapped by chat and topic keys. Weak references allow automatic pruning of idle locks once all strong guards are dropped.
//!
//! ## Search Tags
//! #lock-pool

use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};
use tokio::sync::Mutex as TokioMutex;

/// Type alias `LockKey` (lock key).
pub type LockKey = (i64, Option<i64>);

/// Represents `LockPool` (lock pool).
pub struct LockPool {
    locks: Mutex<HashMap<LockKey, Weak<TokioMutex<()>>>>,
}

impl Default for LockPool {
    fn default() -> Self {
        Self::new()
    }
}

impl LockPool {
    /// Create a new LockPool.
    pub fn new() -> Self {
        Self {
            locks: Mutex::new(HashMap::new()),
        }
    }

    /// Retrieve or create the Lock associated with the key.
    pub fn get(&self, lock_key: LockKey) -> Arc<TokioMutex<()>> {
        let mut locks = self.locks.lock().unwrap_or_else(|e| e.into_inner());

        // Prune dead references first
        locks.retain(|_, weak| weak.strong_count() > 0);

        if let Some(weak) = locks.get(&lock_key)
            && let Some(arc) = weak.upgrade() {
                return arc;
            }

        let new_lock = Arc::new(TokioMutex::new(()));
        locks.insert(lock_key, Arc::downgrade(&new_lock));
        new_lock
    }
}
