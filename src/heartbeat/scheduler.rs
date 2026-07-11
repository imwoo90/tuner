//! # Heartbeat Scheduler Runtime
//!
//! This module coordinates the periodic background check loop.
//! It uses CliConfig settings to determine intervals, quiet hours,
//! and evaluates cooldown conditions before executing the agy CLI.

use std::sync::Arc;
use chrono::{DateTime, NaiveTime, Utc};
use chrono_tz::Tz;
use teloxide::prelude::*;

use crate::config::CliConfig;
use crate::session::manager::SessionManager;
use crate::session::data::SessionData;
use crate::cli::antigravity::AntigravityCli;
use crate::cli::AgentProvider;

pub struct HeartbeatScheduler {
    pub config: Arc<CliConfig>,
    pub sessions: Arc<SessionManager>,
    pub cli: Arc<AntigravityCli>,
}

impl HeartbeatScheduler {
    pub fn new(
        config: Arc<CliConfig>,
        sessions: Arc<SessionManager>,
        cli: Arc<AntigravityCli>,
    ) -> Self {
        Self {
            config,
            sessions,
            cli,
        }
    }

    /// Evaluates if the current tick falls into quiet hours and should be skipped.
    pub fn should_skip_tick(&self, now: &DateTime<Tz>) -> bool {
        if !self.config.telegram_heartbeat_enabled {
            return true;
        }

        if let (Some(start_h), Some(end_h)) = (
            self.config.telegram_heartbeat_quiet_start,
            self.config.telegram_heartbeat_quiet_end,
        ) {
            let start = NaiveTime::from_hms_opt(start_h, 0, 0).unwrap();
            let end = NaiveTime::from_hms_opt(end_h, 0, 0).unwrap();
            return super::quiet::is_within_quiet_hours(now, start, end);
        }

        false
    }

    /// Checks if a given chat session has active running PTY processes.
    pub async fn is_chat_busy(&self, chat_id: i64) -> bool {
        let key = crate::session::key::SessionKey::telegram(chat_id, None);
        if let Ok(Some(sess)) = self.sessions.get_active(&key).await {
            let sid = sess.get_session_id(&self.config.provider);
            if !sid.is_empty() {
                return self.cli.sessions.is_active(&sid).await;
            }
        }
        false
    }

    /// Evaluates if the session activity is still within the cooldown threshold.
    pub fn is_cooling_down(&self, session: &SessionData) -> bool {
        // Cooldown configuration (default 30 minutes if not provided)
        let cooldown_min = self.config.telegram_heartbeat_interval_minutes.unwrap_or(30);
        if let Ok(last) = DateTime::parse_from_rfc3339(&session.last_active) {
            let now = Utc::now();
            let gap = now.signed_duration_since(last.with_timezone(&Utc));
            if gap < chrono::Duration::minutes(cooldown_min) {
                return true;
            }
        }
        false
    }

    /// Starts the background interval check.
    pub fn start(self: Arc<Self>, bot: Bot) {
        let interval_mins = self.config.telegram_heartbeat_interval_minutes.unwrap_or(30) as u64;
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(interval_mins * 60));
            // Skip the immediate first tick
            interval.tick().await;

            loop {
                interval.tick().await;
                let _ = self.tick(&bot).await;
            }
        });
    }

    /// Perform a single tick check over all persisted sessions.
    pub async fn tick(&self, bot: &Bot) -> Result<(), String> {
        let user_tz = self.config.telegram_heartbeat_ack_token.as_ref()
            .map(|_| "UTC".to_string())
            .unwrap_or_else(|| "UTC".to_string());
        
        let tz = user_tz.parse::<Tz>().unwrap_or(Tz::UTC);
        let now_local = Utc::now().with_timezone(&tz);

        if self.should_skip_tick(&now_local) {
            return Ok(());
        }

        let all_sessions = self.sessions.load()?;
        for (_, session) in all_sessions {
            if session.transport != "tg" {
                continue;
            }

            if self.is_chat_busy(session.chat_id).await {
                continue;
            }

            if self.is_cooling_down(&session) {
                continue;
            }

            let _ = self.run_heartbeat_for_chat(bot, &session).await;
        }

        Ok(())
    }

    async fn run_heartbeat_for_chat(&self, bot: &Bot, session: &SessionData) -> Result<(), String> {
        let prompt = "System self-check: are there any outstanding issues or background alerts to report? Answer 'HEARTBEAT_OK' if everything is running smoothly.";
        let ack_token = self.config.telegram_heartbeat_ack_token.as_deref().unwrap_or("HEARTBEAT_OK");

        let sid = session.get_session_id(&self.config.provider);
        let opt_sid = if sid.is_empty() { None } else { Some(&sid[..]) };

        let res = self.cli.send(prompt, opt_sid, false, self.config.working_dir.clone()).await;
        if let Ok(resp) = res {
            if !super::quiet::should_suppress_heartbeat(&resp.result, ack_token) {
                let html_text = crate::telegram::formatting::markdown_to_telegram_html(&resp.result);
                let _ = bot.send_message(ChatId(session.chat_id), html_text)
                    .parse_mode(teloxide::types::ParseMode::Html)
                    .await;
            }
        }
        Ok(())
    }
}
