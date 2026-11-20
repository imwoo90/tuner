//! # Antigravity PTY Initialization & Model Sync Coordinator (`pty_sync.rs`)
//!
//! ## Overview
//! Serializes post-OAuth interactive PTY prompt readiness (`wait_for_pty_prompt`) per
//! session ID, tracks the `/model <target>` applied to each live PTY OS PID, and guards
//! in-flight topic turns and open `ask_question` TUI selectors against concurrent
//! process termination or raw stdin `/model` injection.
//!
//! ## Search Tags
//! #pty-sync, #model-sync, #concurrency-guard, #in-flight-turn

use super::AntigravityCli;
use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex, Weak};
use tokio::sync::Mutex;

type TopicKey = (i64, Option<i64>);

/// Shared synchronization and applied-model registry across cloned `AntigravityCli` handles.
#[derive(Default)]
pub struct PtySyncState {
    sid_locks: Mutex<HashMap<String, Weak<Mutex<()>>>>,
    applied_models: Mutex<HashMap<String, (u32, Option<String>)>>,
    seeded_non_default: Mutex<HashMap<String, u32>>,
    in_flight_topics: StdMutex<HashMap<TopicKey, usize>>,
    fg_end_offsets: Mutex<HashMap<String, u64>>,
    turn_ctx: Mutex<HashMap<String, (u64, Option<String>)>>,
    pub(crate) observer_notify: tokio::sync::Notify,
}

impl PtySyncState {
    /// Returns the per-session async mutex serializing PTY prompt initialization and `/model` writes.
    pub async fn lock_for(&self, sid: &str) -> Arc<Mutex<()>> {
        let mut map = self.sid_locks.lock().await;
        map.retain(|_, weak| weak.strong_count() > 0);
        if let Some(weak) = map.get(sid)
            && let Some(arc) = weak.upgrade()
        {
            return arc;
        }
        let arc = Arc::new(Mutex::new(()));
        map.insert(sid.to_string(), Arc::downgrade(&arc));
        arc
    }

    #[cfg(test)]
    pub(crate) async fn sid_lock_entry_count(&self) -> usize {
        let mut map = self.sid_locks.lock().await;
        map.retain(|_, weak| weak.strong_count() > 0);
        map.len()
    }

    pub(crate) async fn get_applied(&self, sid: &str, pid: u32) -> Option<String> {
        let map = self.applied_models.lock().await;
        map.get(sid)
            .filter(|(p, _)| *p == pid)
            .and_then(|(_, m)| m.clone())
    }

    pub(crate) async fn set_applied(&self, sid: &str, pid: u32, model: Option<String>) {
        self.applied_models
            .lock()
            .await
            .insert(sid.to_string(), (pid, model));
    }

    pub(crate) async fn record_seeded_target(&self, sid: &str, pid: Option<u32>, is_custom: bool) {
        let mut map = self.seeded_non_default.lock().await;
        match (pid, is_custom) {
            (Some(p), true) => {
                map.insert(sid.to_string(), p);
            }
            _ => {
                map.remove(sid);
            }
        }
    }

    pub(crate) async fn has_custom_model(&self, sid: &str, pid: u32) -> bool {
        self.get_applied(sid, pid).await.is_some()
            || self.seeded_non_default.lock().await.get(sid).copied() == Some(pid)
    }

    pub(crate) async fn record_fg_end(&self, sid: &str, offset: u64) {
        self.fg_end_offsets.lock().await.insert(sid.to_string(), offset);
        self.observer_notify.notify_waiters();
    }

    pub(crate) async fn take_fg_end(&self, sid: &str) -> Option<u64> {
        self.fg_end_offsets.lock().await.remove(sid)
    }

    pub(crate) async fn set_turn_ctx(&self, sid: &str, start_pos: u64, pre_bg: Option<String>) {
        self.turn_ctx.lock().await.insert(sid.to_string(), (start_pos, pre_bg));
    }

    pub(crate) async fn take_turn_ctx(&self, sid: &str) -> Option<(u64, Option<String>)> {
        self.turn_ctx.lock().await.remove(sid)
    }

    pub(crate) async fn remove_applied(&self, sid: &str) {
        self.applied_models.lock().await.remove(sid);
        self.seeded_non_default.lock().await.remove(sid);
        self.fg_end_offsets.lock().await.remove(sid);
        self.turn_ctx.lock().await.remove(sid);
        self.sid_locks.lock().await.retain(|_, w| w.strong_count() > 0);
    }

    pub(crate) async fn clear_all_applied(&self) {
        self.applied_models.lock().await.clear();
        self.seeded_non_default.lock().await.clear();
        self.fg_end_offsets.lock().await.clear();
        self.turn_ctx.lock().await.clear();
        self.sid_locks.lock().await.retain(|_, w| w.strong_count() > 0);
    }
}

/// RAII guard tracking an active or resuming foreground/ask turn for `(chat_id, topic_id)`.
pub struct TopicTurnGuard {
    sync: Arc<PtySyncState>,
    key: TopicKey,
}

impl Drop for TopicTurnGuard {
    fn drop(&mut self) {
        if let Ok(mut map) = self.sync.in_flight_topics.lock()
            && let Some(cnt) = map.get_mut(&self.key)
        {
            *cnt = cnt.saturating_sub(1);
            if *cnt == 0 {
                map.remove(&self.key);
            }
        }
    }
}

impl AntigravityCli {
    /// Marks `(chat_id, topic_id)` as having an in-flight turn until the returned guard drops.
    pub fn begin_topic_turn(&self, chat_id: i64, topic_id: Option<i64>) -> TopicTurnGuard {
        let key = (chat_id, topic_id);
        if let Ok(mut map) = self.pty_sync.in_flight_topics.lock() {
            let entry = map.entry(key).or_insert(0);
            *entry = entry.saturating_add(1);
        }
        TopicTurnGuard { sync: self.pty_sync.clone(), key }
    }

    /// Returns `true` when `(chat_id, topic_id)` has an active or resuming turn in flight.
    pub fn is_topic_turn_in_flight(&self, chat_id: i64, topic_id: Option<i64>) -> bool {
        self.pty_sync.in_flight_topics.lock().ok()
            .and_then(|m| m.get(&(chat_id, topic_id)).copied()).unwrap_or(0) > 0
    }

    /// Verifies that the warm process backing an open `ask_question` on `prov` is still alive.
    pub async fn is_ask_backing_alive(
        &self,
        prov: &str,
        chat_id: i64,
        topic_id: Option<i64>,
        sid: &str,
    ) -> bool {
        match crate::session::data::SessionData::canonical_provider(prov).unwrap_or("antigravity") {
            "claude" => crate::cli::claude::has_pending_ask(chat_id, topic_id, Some(sid)).await,
            "codex" => crate::cli::codex::has_pending_ask(chat_id, topic_id, Some(sid)).await,
            _ => sid.starts_with("mock-") || self.sessions.is_active(sid).await,
        }
    }

    /// Evicts stale `active_asks` entries whose warm PTY or peer process has exited or expired.
    pub async fn prune_dead_asks(&self) {
        let sids: Vec<String> = self.sessions.active_asks.lock().await.keys().cloned().collect();
        for sid in sids {
            let alive = sid.starts_with("mock-")
                || self.sessions.is_active(&sid).await
                || crate::cli::claude::has_pending_ask(0, None, Some(&sid)).await
                || crate::cli::codex::has_pending_ask(0, None, Some(&sid)).await;
            if !alive {
                self.sessions.set_ask_active(&sid, false).await;
                self.pty_sync.remove_applied(&sid).await;
            }
        }
    }

    /// Returns `true` if `sid` or its topic is actively running a turn or waiting on `ask_question`.
    pub async fn is_pty_busy_or_asking(
        &self,
        sid: &str,
        chat_id: i64,
        topic_id: Option<i64>,
    ) -> bool {
        if self.is_topic_turn_in_flight(chat_id, topic_id)
            || self.sessions.is_running(sid).await
            || self.sessions.is_async_running(sid).await
        {
            return true;
        }
        if self.sessions.is_ask_active(sid).await {
            self.prune_dead_asks().await;
            return self.sessions.is_ask_active(sid).await;
        }
        false
    }

    /// Terminates `sid` while atomically clearing `active_asks` and cached PTY model state.
    pub async fn terminate_session_clean(&self, sid: &str) {
        self.pty_sync.remove_applied(sid).await;
        self.sessions.set_ask_active(sid, false).await;
        self.sessions.terminate(sid).await;
    }

    /// Clears all cached PTY applied-model entries (used alongside `terminate_all`).
    pub async fn clear_all_pty_applied(&self) {
        self.pty_sync.clear_all_applied().await;
    }
}
