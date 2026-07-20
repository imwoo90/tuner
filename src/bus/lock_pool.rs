//! # Lock Pool Module
//!
//! Provides a lock management system (`LockPool`) that tracks active asynchronous locks
//! mapped by chat and topic keys. Supports automatic eviction of idle locks when size limits are exceeded.

//! 
//! ## Search Tags
//! #lock-pool

use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};
use tokio::sync::Mutex as TokioMutex;

pub type LockKey = (i64, Option<i64>);

pub trait IntoLockKey {
    fn into_lock_key(self) -> LockKey;
}

impl IntoLockKey for i64 {
    fn into_lock_key(self) -> LockKey {
        (self, None)
    }
}

impl IntoLockKey for (i64, Option<i64>) {
    fn into_lock_key(self) -> LockKey {
        self
    }
}

impl IntoLockKey for (i64, i64) {
    fn into_lock_key(self) -> LockKey {
        (self.0, Some(self.1))
    }
}

pub struct LockPool {
    locks: Mutex<HashMap<LockKey, Weak<TokioMutex<()>>>>,
    _max_locks: usize,
}

impl LockPool {
    /// Create a new LockPool with default max locks.
    pub fn new_default() -> Self {
        Self::new(100)
    }

    /// Create a new LockPool with a specific capacity.
    pub fn new(max_locks: usize) -> Self {
        Self {
            locks: Mutex::new(HashMap::new()),
            _max_locks: max_locks,
        }
    }

    /// Retrieve or create the Lock associated with the key.
    pub fn get<K: IntoLockKey>(&self, key: K) -> Arc<TokioMutex<()>> {
        let lock_key = key.into_lock_key();
        let mut locks = self.locks.lock().unwrap();

        // Prune dead references first
        locks.retain(|_, weak| weak.strong_count() > 0);

        if let Some(weak) = locks.get(&lock_key) {
            if let Some(arc) = weak.upgrade() {
                return arc;
            }
        }

        let new_lock = Arc::new(TokioMutex::new(()));
        locks.insert(lock_key, Arc::downgrade(&new_lock));
        new_lock
    }

    /// Helper to get a lock by chat_id with no topic.
    pub fn get_chat(&self, chat_id: i64) -> Arc<TokioMutex<()>> {
        self.get((chat_id, None))
    }

    /// Check if the lock for the given key is currently held.
    pub fn is_locked<K: IntoLockKey>(&self, key: K) -> bool {
        let lock_key = key.into_lock_key();
        let locks = self.locks.lock().unwrap();
        if let Some(weak) = locks.get(&lock_key) {
            if let Some(lock) = weak.upgrade() {
                lock.try_lock().is_err()
            } else {
                false
            }
        } else {
            false
        }
    }

    /// Check if any topic lock is held for the given chat ID.
    pub fn any_locked_for_chat(&self, chat_id: i64) -> bool {
        let locks = self.locks.lock().unwrap();
        locks
            .iter()
            .filter(|((cid, _), _)| *cid == chat_id)
            .any(|(_, weak)| {
                if let Some(lock) = weak.upgrade() {
                    lock.try_lock().is_err()
                } else {
                    false
                }
            })
    }

    /// Return the number of locks currently tracked in the pool.
    pub fn len(&self) -> usize {
        let mut locks = self.locks.lock().unwrap();
        locks.retain(|_, weak| weak.strong_count() > 0);
        locks.len()
    }
}
