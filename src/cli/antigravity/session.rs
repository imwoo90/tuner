//! # Antigravity Session Holder and PTY Spawning
//!
//! This module manages active interactive sessions with the Google Antigravity CLI.
//! It handles non-echoing PTY spawning, non-blocking asynchronous output draining,
//! and automatic resource cleanup to prevent zombie processes.

use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;
use tokio::sync::Mutex;
pub use super::pty_spawner::spawn_session;
use super::pty_spawner::SessionHolder;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct AskState {
    pub msg_id: i32,
    pub questions: Vec<crate::cli::AskQuestionData>,
    pub current_index: usize,
    pub answers: Vec<String>,
    pub current_bitmap: String,
    pub waiting_for_write_in: bool,
}

pub struct SessionManager {
    pub(crate) holders: Mutex<HashMap<String, SessionHolder>>,
    pub(crate) running_runs: Mutex<std::collections::HashSet<String>>,
    pub(crate) active_asks: Mutex<HashMap<String, AskState>>,
}

impl Default for SessionManager {
    fn default() -> Self {
        Self {
            holders: Mutex::new(HashMap::new()),
            running_runs: Mutex::new(std::collections::HashSet::new()),
            active_asks: Mutex::new(HashMap::new()),
        }
    }
}

fn terminate_duplicates(
    sid: &str,
    cid: Option<i64>,
    tid: Option<i64>,
    holders: &mut HashMap<String, SessionHolder>,
) {
    if let Some(c) = cid {
        holders.retain(|id, h| id == sid || h.chat_id != Some(c) || h.topic_id != tid);
    }
}

impl SessionManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn is_running(&self, session_id: &str) -> bool {
        let runs = self.running_runs.lock().await;
        runs.contains(session_id)
    }

    pub async fn set_running(&self, session_id: &str, running: bool) {
        let mut runs = self.running_runs.lock().await;
        if running {
            runs.insert(session_id.to_string());
        } else {
            runs.remove(session_id);
        }
    }

    pub async fn is_ask_active(&self, session_id: &str) -> bool {
        let asks = self.active_asks.lock().await;
        asks.contains_key(session_id)
    }

    pub async fn set_ask_active(&self, session_id: &str, active: bool) {
        let mut asks = self.active_asks.lock().await;
        if active {
            asks.entry(session_id.to_string()).or_insert_with(|| AskState {
                msg_id: 0,
                questions: Vec::new(),
                current_index: 0,
                answers: Vec::new(),
                current_bitmap: String::new(),
                waiting_for_write_in: false,
            });
        } else {
            asks.remove(session_id);
        }
    }

    pub async fn set_ask_state(&self, session_id: &str, state: AskState) {
        let mut asks = self.active_asks.lock().await;
        asks.insert(session_id.to_string(), state);
    }

    pub async fn get_ask_state(&self, session_id: &str) -> Option<AskState> {
        let asks = self.active_asks.lock().await;
        asks.get(session_id).cloned()
    }

    pub async fn cleanup_expired(&self) {
        let now = Instant::now();
        self.holders.lock().await.retain(|_, h| {
            h.child.try_wait().map(|s| s.is_none()).unwrap_or(false)
                && now.duration_since(h.last_active) < std::time::Duration::from_secs(86400)
        });
    }

    pub async fn ensure_session(
        &self,
        session_id: &str,
        workspace: &Path,
        cmd_name: &str,
        cmd_args: &[String],
        env: &HashMap<String, String>,
    ) -> Result<(), String> {
        self.cleanup_expired().await;

        let chat_id = env.get("TUNER_CHAT_ID").and_then(|s| s.parse::<i64>().ok());
        let topic_id = env.get("TUNER_TOPIC_ID").and_then(|s| s.parse::<i64>().ok());

        let mut holders = self.holders.lock().await;
        let is_running = holders.get_mut(session_id)
            .map(|h| h.child.try_wait().map(|s| s.is_none()).unwrap_or(false))
            .unwrap_or(false);

        if is_running {
            terminate_duplicates(session_id, chat_id, topic_id, &mut holders);
            if let Some(h) = holders.get_mut(session_id) { h.last_active = Instant::now(); }
            return Ok(());
        }
        holders.remove(session_id);

        terminate_duplicates(session_id, chat_id, topic_id, &mut holders);

        let holder = spawn_session(workspace, cmd_name, cmd_args, env)?;
        holders.insert(session_id.to_string(), holder);
        Ok(())
    }

    pub async fn terminate(&self, session_id: &str) -> bool {
        self.holders.lock().await.remove(session_id).is_some()
    }

    pub async fn abort(&self, chat_id: i64, topic_id: Option<i64>) -> usize {
        let mut h = self.holders.lock().await;
        let prev = h.len();
        h.retain(|_, val| val.chat_id != Some(chat_id) || val.topic_id != topic_id);
        prev - h.len()
    }

    pub async fn terminate_all(&self) {
        self.holders.lock().await.clear();
    }

    pub async fn write_to_session(&self, session_id: &str, input: &str) -> Result<bool, String> {
        let holders = self.holders.lock().await;
        if let Some(holder) = holders.get(session_id) {
            for c in input.chars() {
                let s = c.to_string();
                holder.write_input(&s)?;
                tokio::time::sleep(tokio::time::Duration::from_millis(40)).await;
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub async fn is_active(&self, session_id: &str) -> bool {
        let mut h = self.holders.lock().await;
        if let Some(holder) = h.get_mut(session_id) {
            if holder.child.try_wait().map(|s| s.is_none()).unwrap_or(false) { return true; }
            h.remove(session_id);
        }
        false
    }
}

