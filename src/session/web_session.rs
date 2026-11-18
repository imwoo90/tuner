//! # Web-Native Session Allocation & History Pruning (`web_session.rs`)
//!
//! ## Overview
//! Extends [`crate::session::manager::SessionManager`] with atomic monotonic `web:0:<tid>`
//! topic creation, single-session removal, and unpinned inactive/empty session pruning.
//!
//! ## Search Tags
//! #web-session, #topic-allocation, #session-pruning

use super::create_seeded_session;
use super::data::SessionData;
use super::key::SessionKey;
use super::manager::SessionManager;
use chrono::{DateTime, Duration, Utc};

impl SessionManager {
    /// Atomically allocates the next monotonic `web:0:<tid>` session and persists it.
    pub async fn create_web_session(
        &self,
        topic_name: Option<String>,
        provider: &str,
        model: &str,
        effort: Option<&str>,
    ) -> Result<SessionData, String> {
        let _guard = self.lock.lock().await;
        let mut map = self.load()?;
        let next_tid = map
            .values()
            .filter(|s| s.transport == "web" && s.chat_id == 0)
            .filter_map(|s| s.topic_id)
            .max()
            .unwrap_or(0)
            .saturating_add(1)
            .max(1);
        let key = SessionKey::web(next_tid);
        let mut sess = create_seeded_session(&key, provider, model, topic_name);
        if let Some(eff) = effort.filter(|e| !e.trim().is_empty()) {
            sess.effort = Some(eff.trim().to_ascii_lowercase());
            let p = sess.provider.clone();
            sess.normalize_effort_for_provider(&p);
        }
        map.insert(key.storage_key(), sess.clone());
        self.save(&map)?;
        Ok(sess)
    }

    /// Removes a persisted `web` or `tg` session entry from `sessions.json`.
    pub async fn remove_session(&self, key: &SessionKey) -> Result<bool, String> {
        let _guard = self.lock.lock().await;
        let mut map = self.load()?;
        let removed = map.remove(&key.storage_key()).is_some();
        if removed {
            self.save(&map)?;
        }
        Ok(removed)
    }

    /// Prunes unpinned empty or stale sessions matching `transport` (`web` or `tg`).
    pub async fn prune_inactive_sessions(
        &self,
        transport: &str,
        older_than_days: i64,
    ) -> Result<Vec<SessionKey>, String> {
        let _guard = self.lock.lock().await;
        let mut map = self.load()?;
        let cutoff = Utc::now() - Duration::days(older_than_days.max(1));
        let mut pruned = Vec::new();
        map.retain(|_, s| {
            if s.transport != transport || s.pinned {
                return true;
            }
            let total_msgs: i64 = s.provider_sessions.values().map(|p| p.message_count).sum();
            let has_history = total_msgs > 0
                || s.provider_sessions.values().any(|p| !p.past_session_ids.is_empty());
            let is_stale = DateTime::parse_from_rfc3339(&s.last_active)
                .map(|dt| dt.with_timezone(&Utc) < cutoff)
                .unwrap_or(false);
            if !has_history || is_stale {
                pruned.push(s.session_key());
                return false;
            }
            true
        });
        if !pruned.is_empty() {
            self.save(&map)?;
        }
        Ok(pruned)
    }
}
