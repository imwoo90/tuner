//! # Heartbeat Telemetry Loop
//!
//! ## Overview
//! Ticks periodically to log daemon health status and telemetry parameters.
//!
//! ## Search Tags
//! #heartbeat-loop, #telemetry-logger, #daemon-health

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use chrono::{DateTime, NaiveTime, Utc};
use chrono_tz::Tz;

use crate::config::CliConfig;
use crate::session::manager::SessionManager;
use crate::session::data::SessionData;
use crate::cli::antigravity::AntigravityCli;
use crate::cli::AgentProvider;

/// Represents `HeartbeatScheduler` (heartbeat scheduler).
pub struct HeartbeatScheduler {
    /// Config.
    pub config: Arc<CliConfig>,
    /// Sessions.
    pub sessions: Arc<SessionManager>,
    /// Cli.
    pub cli: Arc<AntigravityCli>,
    /// Bus.
    pub bus: Arc<crate::bus::bus::MessageBus>,
    last_heartbeat_at: Mutex<HashMap<String, DateTime<Utc>>>,
}

impl HeartbeatScheduler {
    /// Creates a new instance.
    pub fn new(
        config: Arc<CliConfig>,
        sessions: Arc<SessionManager>,
        cli: Arc<AntigravityCli>,
        bus: Arc<crate::bus::bus::MessageBus>,
    ) -> Self {
        Self {
            config,
            sessions,
            cli,
            bus,
            last_heartbeat_at: Mutex::new(HashMap::new()),
        }
    }

    /// Executes `heartbeat_enabled` (heartbeat enabled).
    pub fn heartbeat_enabled(&self) -> bool {
        self.config.heartbeat.enabled
    }

    /// Executes `heartbeat_interval_minutes` (heartbeat interval minutes).
    pub fn heartbeat_interval_minutes(&self) -> i64 {
        self.config.heartbeat.interval_minutes.unwrap_or(30)
    }

    /// Executes `heartbeat_quiet_start` (heartbeat quiet start).
    pub fn heartbeat_quiet_start(&self) -> Option<u32> {
        self.config.heartbeat.quiet_start
    }

    /// Executes `heartbeat_quiet_end` (heartbeat quiet end).
    pub fn heartbeat_quiet_end(&self) -> Option<u32> {
        self.config.heartbeat.quiet_end
    }

    /// Executes `heartbeat_ack_token` (heartbeat ack token).
    pub fn heartbeat_ack_token(&self) -> String {
        self.config
            .heartbeat
            .ack_token
            .clone()
            .unwrap_or_else(|| "HEARTBEAT_OK".to_string())
    }

    /// Evaluates if the current tick falls into quiet hours and should be skipped.
    pub fn should_skip_tick(&self, now: &DateTime<Tz>) -> bool {
        if !self.heartbeat_enabled() {
            return true;
        }

        if let (Some(start_h), Some(end_h)) = (
            self.heartbeat_quiet_start(),
            self.heartbeat_quiet_end(),
        ) {
            let start = NaiveTime::from_hms_opt(start_h % 24, 0, 0).unwrap_or_default();
            let end = NaiveTime::from_hms_opt(end_h % 24, 0, 0).unwrap_or_default();
            return super::quiet::is_within_quiet_hours(now, start, end);
        }

        false
    }

    /// Checks if a given chat/topic session has active running CLI processes or open `ask_question` prompts.
    pub async fn is_session_busy(&self, chat_id: i64, topic_id: Option<i64>) -> bool {
        if self.cli.is_topic_turn_in_flight(chat_id, topic_id)
            || self.cli.sessions.boot_cancels.lock().await.contains_key(&(chat_id, topic_id))
        {
            return true;
        }
        let key = crate::session::key::SessionKey::telegram(chat_id, topic_id);
        if let Ok(Some(sess)) = self.sessions.get_active(&key).await {
            let prov = self.effective_provider(&sess);
            let sid = sess.get_session_id(prov);
            let opt_sid = (!sid.is_empty()).then_some(sid.as_str());
            if !sid.is_empty() && self.cli.is_pty_busy_or_asking(&sid, chat_id, topic_id).await {
                return true;
            }
            return match prov {
                "claude" => {
                    crate::cli::claude::is_running(chat_id, topic_id, opt_sid).await
                        || crate::cli::claude::has_pending_ask(chat_id, topic_id, opt_sid).await
                }
                "codex" => {
                    crate::cli::codex::is_running(chat_id, topic_id, opt_sid).await
                        || crate::cli::codex::has_pending_ask(chat_id, topic_id, opt_sid).await
                }
                _ => !sid.is_empty() && self.cli.sessions.is_active(&sid).await,
            };
        }
        false
    }

    fn effective_provider<'a>(&'a self, session: &'a SessionData) -> &'static str {
        let raw = if session.provider.trim().is_empty() {
            self.config.provider.as_str()
        } else {
            session.provider.as_str()
        };
        SessionData::canonical_provider(raw).unwrap_or("antigravity")
    }

    /// Returns `true` only when `session` already has a persisted conversation ID on its active
    /// provider and has not exceeded idle, daily-reset, or message-count freshness limits.
    pub(crate) fn has_fresh_active_session(&self, session: &SessionData) -> bool {
        let eff_prov = self.effective_provider(session);
        !session.get_session_id(eff_prov).is_empty()
            && self.sessions.is_fresh_for_provider(session, eff_prov)
    }

    fn record_heartbeat_timestamp(&self, session: &SessionData) {
        let mut map = self.last_heartbeat_at.lock().unwrap_or_else(|e| e.into_inner());
        map.insert(session.session_key().storage_key(), Utc::now());
    }

    /// Evaluates if the session activity or last heartbeat is still within the cooldown threshold.
    pub fn is_cooling_down(&self, session: &SessionData) -> bool {
        let cooldown = chrono::Duration::minutes(self.heartbeat_interval_minutes());
        let now = Utc::now();
        if let Ok(last) = DateTime::parse_from_rfc3339(&session.last_active)
            && now.signed_duration_since(last.with_timezone(&Utc)) < cooldown
        {
            return true;
        }
        let map = self.last_heartbeat_at.lock().unwrap_or_else(|e| e.into_inner());
        map.get(&session.session_key().storage_key())
            .is_some_and(|last_hb| now.signed_duration_since(*last_hb) < cooldown)
    }

    /// Starts the background interval check.
    pub fn start(self: Arc<Self>) {
        let interval_mins = self.heartbeat_interval_minutes() as u64;
        let scheduler = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(interval_mins * 60));
            interval.tick().await;

            loop {
                interval.tick().await;
                let _ = scheduler.tick().await;
            }
        });
    }

    /// Perform a single tick check over all persisted sessions.
    pub async fn tick(&self) -> Result<(), String> {
        let user_tz = self.config.user_timezone.as_deref().unwrap_or("UTC");
        let tz = user_tz.parse::<Tz>().unwrap_or(Tz::UTC);
        let now_local = Utc::now().with_timezone(&tz);

        if self.should_skip_tick(&now_local) {
            return Ok(());
        }

        let all_sessions = self.sessions.load()?;
        for (_, session) in all_sessions {
            if session.transport != "tg" || !self.has_fresh_active_session(&session) {
                continue;
            }

            if self.is_cooling_down(&session) || self.is_session_busy(session.chat_id, session.topic_id).await {
                continue;
            }

            let _ = self.run_heartbeat_for_chat(&session).await;
        }

        Ok(())
    }

    pub(crate) fn build_heartbeat_cli_config(&self, session: &SessionData) -> CliConfig {
        super::build_heartbeat_cli_config(&self.config, session)
    }

    pub(crate) async fn finish_heartbeat_turn(
        &self,
        cur: &SessionData,
        eff_prov: &str,
        start_sid: &str,
        resp: &crate::cli::CliResponse,
    ) {
        self.record_heartbeat_timestamp(cur);
        let ack = self.heartbeat_ack_token();
        if !super::should_deliver_heartbeat_response(cur.chat_id, cur.topic_id, start_sid, &ack, resp) {
            return;
        }
        let prov_key = SessionData::canonical_provider(eff_prov).unwrap_or("antigravity");
        let fallback = start_sid.is_empty().then_some(cur);
        let mut active = false;
        let _ = self
            .sessions
            .mutate_session(&cur.session_key(), fallback, |latest| {
                super::reconcile_heartbeat_session(
                    latest,
                    prov_key,
                    start_sid,
                    resp.session_id.as_deref(),
                    &mut active,
                )
            })
            .await;
        if active {
            crate::upgrade::record_user_activity();
            let mut env = crate::bus::adapters::from_heartbeat(cur.chat_id, &resp.result, cur.topic_id, Some(&cur.transport));
            self.bus.submit(&mut env).await;
        }
    }

    async fn run_heartbeat_for_chat(&self, session: &SessionData) -> Result<(), String> {
        let lock = self.sessions.lock_pool.get((session.chat_id, session.topic_id));
        let Ok(_topic_guard) = lock.try_lock() else { return Ok(()); };
        if self.is_session_busy(session.chat_id, session.topic_id).await {
            return Ok(());
        }
        let Some(cur) = self.sessions.get_active(&session.session_key()).await.ok().flatten() else {
            return Ok(());
        };
        if !self.has_fresh_active_session(&cur) || self.is_cooling_down(&cur) {
            return Ok(());
        }
        self.record_heartbeat_timestamp(&cur);
        let _bg_guard = crate::upgrade::begin_background_run();
        let _turn_guard = self.cli.begin_topic_turn(cur.chat_id, cur.topic_id);
        let prompt = "System self-check: are there any outstanding issues or background alerts to report? Answer 'HEARTBEAT_OK' if everything is running smoothly.";
        let eff_prov = self.effective_provider(&cur);
        if !matches!(eff_prov, "claude" | "codex") {
            crate::cli::antigravity::model_spec::ensure_live_catalog_fresh().await;
        }
        let sid = cur.get_session_id(eff_prov);
        let cli = AntigravityCli::new_oneshot(self.build_heartbeat_cli_config(&cur));
        match cli.send(prompt, Some(&sid), false, self.config.working_dir.clone()).await {
            Ok(resp) => self.finish_heartbeat_turn(&cur, eff_prov, &sid, &resp).await,
            Err(e) => eprintln!("⚠️ [tuner] Heartbeat send failed for chat {} topic {:?}: {e}", cur.chat_id, cur.topic_id),
        }
        Ok(())
    }
}
