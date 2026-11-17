//! # Web-Native Session Allocation (`web_session.rs`)
//!
//! ## Overview
//! Extends [`crate::session::manager::SessionManager`] with atomic monotonic `web:0:<tid>`
//! topic creation under the persisted session mutex.
//!
//! ## Search Tags
//! #web-session, #topic-allocation

use super::create_seeded_session;
use super::data::SessionData;
use super::key::SessionKey;
use super::manager::SessionManager;

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

    /// Removes a Web-native (`web:0:<tid>`) session entry from persisted storage.
    pub async fn delete_web_session(&self, key: &SessionKey) -> Result<bool, String> {
        if key.transport != "web" || key.chat_id != 0 {
            return Err("Only web-native topics can be deleted from the dashboard".to_string());
        }
        let _guard = self.lock.lock().await;
        let mut map = self.load()?;
        let removed = map.remove(&key.storage_key()).is_some();
        if removed {
            self.save(&map)?;
        }
        Ok(removed)
    }
}
