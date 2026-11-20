//! # Observer Descendant Subagent Stream Helper (`observer_subagents.rs`)
//!
//! ## Overview
//! Streams live incremental progress from descendant Antigravity subagents and Native Swarm
//! workers to Telegram (single in-place status message) and the Web Dashboard without holding
//! the parent topic's foreground `LockPool` mutex while the parent Supervisor sits idle.
//!
//! ## Search Tags
//! #subagent-stream, #async-observer, #live-progress

use crate::cli::antigravity::log_parser::AntigravityLogParser;
use crate::cli::antigravity::subagent_progress::SubagentProgressTracker;
use crate::cli::antigravity::AntigravityCli;
use crate::messenger::web::events_bridge as wev;
use crate::session::data::SessionData;
use std::path::Path;
use std::time::{Duration, Instant};
use teloxide::prelude::*;
use teloxide::types::MessageId;

/// Builds a synthetic `antigravity` [`SessionData`] snapshot for background observer asks.
pub(crate) fn build_observer_ask_session(
    active: Option<SessionData>,
    chat_id: i64,
    topic_id: Option<i64>,
    agy_sid: &str,
) -> SessionData {
    let mut sess_data = active.unwrap_or_else(|| SessionData {
        chat_id,
        topic_id,
        ..Default::default()
    });
    sess_data.provider = "antigravity".to_string();
    sess_data.set_session_id("antigravity", agy_sid);
    sess_data
}

/// Per-session state tracking live Telegram & Web progress for descendant subagents.
pub(crate) struct ObserverSubagentState {
    tracker: SubagentProgressTracker,
    pub_msg_id: Option<MessageId>,
    last_edit: Instant,
    last_text: String,
}

impl ObserverSubagentState {
    /// Creates a new state container.
    pub(crate) fn new() -> Self {
        Self {
            tracker: SubagentProgressTracker::new(),
            pub_msg_id: None,
            last_edit: Instant::now(),
            last_text: String::new(),
        }
    }

    /// Resets live message tracking when a foreground user turn executes on the parent session.
    pub(crate) fn clear_on_foreground(&mut self) {
        self.tracker.reset();
        self.pub_msg_id = None;
        self.last_text.clear();
    }

    /// Hands off any live subagent Telegram message ID to the waking parent async turn for in-place replacement.
    pub(crate) fn take_adoptable_msg_id(&mut self) -> Option<MessageId> {
        self.tracker.reset();
        self.last_text.clear();
        self.pub_msg_id.take()
    }

    /// Synchronizes foreground-to-background cursor handoff without dropping immediate post-turn bytes.
    pub(crate) async fn sync_running_state(
        &mut self,
        cli: &AntigravityCli,
        session_id: &str,
        transcript_path: &Path,
        was_running: &mut bool,
        last_size: &mut u64,
        parser: &mut AntigravityLogParser,
    ) -> bool {
        let is_running = cli.sessions.is_running(session_id).await;
        let recent_int = cli.sessions.was_recently_interrupted(session_id, Duration::from_secs(6)).await;
        if is_running || recent_int {
            *was_running = true;
            self.clear_on_foreground();
            cli.sessions.set_async_running(session_id, false).await;
            *last_size = std::fs::metadata(transcript_path).map(|m| m.len()).unwrap_or(*last_size);
            *parser = AntigravityLogParser::new();
            return true;
        }
        if *was_running {
            *was_running = false;
            cli.sessions.set_async_running(session_id, false).await;
            *last_size = cli.pty_sync.take_fg_end(session_id).await.unwrap_or(*last_size);
            *parser = AntigravityLogParser::new();
        }
        false
    }

    /// Polls descendant subagent transcripts and dispatches throttled deltas to Web & Telegram.
    pub(crate) async fn poll_and_stream(
        &mut self,
        bot: Option<&Bot>,
        transport: &str,
        chat_id: ChatId,
        thread_id: Option<i32>,
        session_id: &str,
        brain_root: &Path,
    ) {
        let Some(preview) = self.tracker.poll_delta(brain_root, session_id) else {
            return;
        };
        crate::upgrade::record_user_activity();
        wev::obs_delta(transport, chat_id.0, thread_id.map(|t| t as i64), session_id, &preview).await;
        if let Some(b) = bot {
            let _ = super::stream::handle_text_delta(
                b,
                chat_id,
                thread_id,
                &preview,
                &mut self.last_text,
                &mut self.pub_msg_id,
                &mut self.last_edit,
            )
            .await;
        }
    }
}
