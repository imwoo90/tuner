//! # Antigravity Session Holder and PTY Spawning
//!
//! ## Overview
//! This module manages active interactive sessions with the Google Antigravity CLI.
//! It handles non-echoing PTY spawning, non-blocking asynchronous output draining,
//! and automatic resource cleanup to prevent zombie processes.
//!
//! ## Search Tags
//! #session

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
pub use super::pty_spawner::spawn_session;
use super::pty_spawner::SessionHolder;

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
/// Represents `AskState` (ask state).
pub struct AskState {
    /// Msg id.
    pub msg_id: i32,
    /// Questions.
    pub questions: Vec<crate::cli::AskQuestionData>,
    /// Current index.
    pub current_index: usize,
    /// Answers.
    pub answers: Vec<String>,
    /// Current bitmap.
    pub current_bitmap: String,
    /// Waiting for write in.
    pub waiting_for_write_in: bool,
}

#[derive(Default)]
/// Represents `SessionManager` (session manager).
pub struct SessionManager {
    pub(crate) holders: Mutex<HashMap<String, SessionHolder>>,
    pub(crate) running_runs: Mutex<HashSet<String>>,
    pub(crate) async_runs: Mutex<HashSet<String>>,
    pub(crate) active_asks: Mutex<HashMap<String, AskState>>,
    pub(crate) interrupted_sessions: Mutex<HashSet<String>>,
    pub(crate) interrupted_at: Mutex<HashMap<String, Instant>>,
    pub(crate) interrupt_notify: tokio::sync::Notify,
    #[allow(clippy::type_complexity)]
    pub(crate) boot_cancels: Mutex<HashMap<(i64, Option<i64>), tokio::sync::oneshot::Sender<()>>>,
}

fn terminate_duplicates(
    sid: &str,
    cid: Option<i64>,
    tid: Option<i64>,
    holders: &mut HashMap<String, SessionHolder>,
) {
    if let Some(c) = cid.filter(|&c| c != 0) {
        holders.retain(|id, h| id == sid || h.chat_id != Some(c) || h.topic_id != tid);
    }
}

impl SessionManager {
    /// Creates a new instance.
    pub fn new() -> Self {
        Self::default()
    }

    /// Executes `active_count` (active count).
    pub async fn active_count(&self) -> usize {
        self.holders.lock().await.len()
    }

    /// Executes `is_running` (is running).
    pub async fn is_running(&self, session_id: &str) -> bool {
        self.running_runs.lock().await.contains(session_id)
    }

    /// Executes `set_running` (set running).
    pub async fn set_running(&self, session_id: &str, running: bool) {
        let mut runs = self.running_runs.lock().await;
        if running {
            runs.insert(session_id.to_string());
            self.interrupted_at.lock().await.remove(session_id);
        } else {
            runs.remove(session_id);
        }
    }

    /// Executes `set_async_running` (set async running).
    pub async fn set_async_running(&self, sid: &str, running: bool) {
        let mut runs = self.async_runs.lock().await;
        if running { runs.insert(sid.to_string()); } else { runs.remove(sid); }
    }

    /// Executes `is_ask_active` (is ask active).
    pub async fn is_ask_active(&self, session_id: &str) -> bool {
        self.active_asks.lock().await.contains_key(session_id)
    }

    /// Executes `set_ask_active` (set ask active).
    pub async fn set_ask_active(&self, session_id: &str, active: bool) {
        let mut asks = self.active_asks.lock().await;
        if active {
            asks.entry(session_id.to_string()).or_default();
        } else {
            asks.remove(session_id);
        }
    }

    /// Executes `set_ask_state` (set ask state).
    pub async fn set_ask_state(&self, session_id: &str, state: AskState) {
        self.active_asks.lock().await.insert(session_id.to_string(), state);
    }

    /// Executes `get_ask_state` (get ask state).
    pub async fn get_ask_state(&self, session_id: &str) -> Option<AskState> {
        self.active_asks.lock().await.get(session_id).cloned()
    }

    /// Executes `cleanup_expired` (cleanup expired).
    pub async fn cleanup_expired(&self) {
        let now = Instant::now();
        self.holders.lock().await.retain(|_, h| {
            h.child.try_wait().map(|s| s.is_none()).unwrap_or(false)
                && now.duration_since(h.last_active) < Duration::from_secs(86400)
        });
    }

    /// Executes `ensure_session` (ensure session).
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

    /// Executes `terminate` (terminate).
    pub async fn terminate(&self, session_id: &str) -> bool {
        self.interrupted_sessions.lock().await.remove(session_id);
        self.holders.lock().await.remove(session_id).is_some()
    }

    /// Executes `register_boot` (register boot).
    pub async fn register_boot(&self, chat_id: i64, topic_id: Option<i64>) -> tokio::sync::oneshot::Receiver<()> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.boot_cancels.lock().await.insert((chat_id, topic_id), tx);
        rx
    }

    /// Executes `unregister_boot` (unregister boot).
    pub async fn unregister_boot(&self, chat_id: i64, topic_id: Option<i64>) {
        self.boot_cancels.lock().await.remove(&(chat_id, topic_id));
    }

    /// Executes `abort` (abort).
    pub async fn abort(&self, chat_id: i64, topic_id: Option<i64>) -> usize {
        if let Some(tx) = self.boot_cancels.lock().await.remove(&(chat_id, topic_id)) {
            let _ = tx.send(());
        }
        let mut h = self.holders.lock().await;
        let prev = h.len();
        h.retain(|_, val| val.chat_id != Some(chat_id) || val.topic_id != topic_id);
        prev - h.len()
    }

    /// Executes `terminate_all` (terminate all).
    pub async fn terminate_all(&self) {
        let mut cancels = self.boot_cancels.lock().await;
        for (_, tx) in cancels.drain() {
            let _ = tx.send(());
        }
        self.interrupted_sessions.lock().await.clear();
        self.holders.lock().await.clear();
    }

    /// Executes `interrupt` (interrupt).
    pub async fn interrupt(&self, chat_id: i64, topic_id: Option<i64>) -> bool {
        let mut found = self.boot_cancels.lock().await.remove(&(chat_id, topic_id))
            .is_some_and(|tx| { let _ = tx.send(()); true });

        let matching: Vec<_> = self.holders.lock().await.iter()
            .filter(|(_, h)| h.chat_id == Some(chat_id) && h.topic_id == topic_id)
            .map(|(sid, h)| (sid.clone(), h.master_fd)).collect();

        let mut sids = Vec::new();
        for (sid, fd) in matching {
            let is_run = self.is_running(&sid).await || self.async_runs.lock().await.remove(&sid);
            let is_ask = self.is_ask_active(&sid).await;
            if is_run || is_ask {
                let _ = super::pty_spawner::write_fd(fd, "\x1b");
                if is_run && !is_ask && !cfg!(test) {
                    tokio::spawn(async move {
                        tokio::time::sleep(Duration::from_millis(150)).await;
                        let _ = super::pty_spawner::write_fd(fd, "\x1b");
                    });
                }
                if is_run { sids.push(sid.clone()); }
                if is_ask { self.set_ask_active(&sid, false).await; found = true; }
            }
        }

        if !sids.is_empty() {
            let now = Instant::now();
            let (mut int_set, mut ts) = (self.interrupted_sessions.lock().await, self.interrupted_at.lock().await);
            for sid in sids { ts.insert(sid.clone(), now); int_set.insert(sid); }
            drop((int_set, ts));
            self.interrupt_notify.notify_waiters();
            found = true;
        }
        found
    }

    /// Executes `was_recently_interrupted` (was recently interrupted).
    pub async fn was_recently_interrupted(&self, sid: &str, within: std::time::Duration) -> bool {
        self.interrupted_at.lock().await.get(sid).is_some_and(|t| t.elapsed() < within)
    }

    /// Executes `is_interrupted` (is interrupted).
    pub async fn is_interrupted(&self, session_id: &str) -> bool {
        let interrupted = self.interrupted_sessions.lock().await;
        interrupted.contains(session_id)
    }

    /// Executes `clear_interrupted` (clear interrupted).
    pub async fn clear_interrupted(&self, session_id: &str) {
        let mut interrupted = self.interrupted_sessions.lock().await;
        interrupted.remove(session_id);
    }

    /// Executes `write_to_session` (write to session).
    pub async fn write_to_session(&self, session_id: &str, input: &str) -> Result<bool, String> {
        let master_fd = {
            let holders = self.holders.lock().await;
            holders.get(session_id).map(|h| h.master_fd)
        };

        if let Some(fd) = master_fd {
            if input.contains('\x7f') {
                for c in input.chars() {
                    let s = c.to_string();
                    super::pty_spawner::write_fd(fd, &s)?;
                    let delay = if cfg!(test) {
                        tokio::time::Duration::from_millis(1)
                    } else {
                        tokio::time::Duration::from_millis(20)
                    };
                    tokio::time::sleep(delay).await;
                }
            } else {
                super::pty_spawner::write_fd(fd, input)?;
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Executes `is_active` (is active).
    pub async fn is_active(&self, session_id: &str) -> bool {
        let mut h = self.holders.lock().await;
        if let Some(holder) = h.get_mut(session_id) {
            return holder.child.try_wait().map(|s| s.is_none()).unwrap_or(false);
        }
        false
    }

    /// Executes `get_output_len` (get output len).
    pub async fn get_output_len(&self, session_id: &str) -> Option<usize> {
        let holders = self.holders.lock().await;
        let holder = holders.get(session_id)?;
        let out = holder.output.lock().await;
        Some(out.len())
    }

    /// Executes `get_output_from` (get output from).
    pub async fn get_output_from(&self, session_id: &str, from: usize) -> Option<Vec<u8>> {
        let holders = self.holders.lock().await;
        let holder = holders.get(session_id)?;
        let out = holder.output.lock().await;
        if from <= out.len() {
            Some(out[from..].to_vec())
        } else {
            Some(Vec::new())
        }
    }
}

