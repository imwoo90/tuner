//! # Session Freshness Checker
//!
//! ## Overview
//! Implements logical rules to verify if an active session is still fresh, checking inactivity timeouts,
//! max message limits, and daily user-timezone resets.
//!
//! ## Collaboration Graph
//! - Called by [`SessionManager::is_fresh`](super::manager::SessionManager::is_fresh) on new message ingress.
//!
//! ## Search Tags
//! #session-freshness, #timeout-inactivity, #daily-resets, #timezone-calculator

use chrono::{DateTime, Utc, Duration, LocalResult, TimeZone};
use chrono_tz::Tz;
use crate::session::data::SessionData;

/// Evaluate whether the given session has crossed the timezone-based daily reset threshold.
pub fn has_crossed_daily_reset(
    last_active: &DateTime<Utc>,
    now: &DateTime<Utc>,
    user_timezone: &str,
    daily_reset_hour: u32,
) -> bool {
    let tz = user_timezone.parse::<Tz>().unwrap_or(Tz::UTC);
    let now_local = now.with_timezone(&tz);
    let last_local = last_active.with_timezone(&tz);

    let today_reset = match tz.from_local_datetime(
        &now_local.date_naive().and_hms_opt(daily_reset_hour, 0, 0).unwrap()
    ) {
        LocalResult::Single(dt) => dt,
        LocalResult::Ambiguous(dt1, _) => dt1,
        LocalResult::None => return true, // Expire on error
    };

    if now_local >= today_reset {
        last_local < today_reset
    } else {
        let yesterday_reset = today_reset - Duration::days(1);
        last_local < yesterday_reset
    }
}

/// Check if the session is fresh according to max messages, idle timeout, and daily reset rules.
pub fn is_session_fresh(
    session: &SessionData,
    max_session_messages: Option<i64>,
    idle_timeout_minutes: i64,
    daily_reset_enabled: bool,
    daily_reset_hour: u32,
    user_timezone: &str,
) -> bool {
    let now = Utc::now();
    let last = match DateTime::parse_from_rfc3339(&session.last_active) {
        Ok(dt) => dt.with_timezone(&Utc),
        Err(_) => return false,
    };

    if let Some(max_msg) = max_session_messages {
        let current_provider_ps = session.provider_sessions.get(&session.provider);
        let msg_count = current_provider_ps.map(|ps| ps.message_count).unwrap_or(0);
        if msg_count >= max_msg {
            return false;
        }
    }

    if idle_timeout_minutes > 0 && now.signed_duration_since(last) >= Duration::minutes(idle_timeout_minutes) {
        return false;
    }

    if daily_reset_enabled && has_crossed_daily_reset(&last, &now, user_timezone, daily_reset_hour) {
        return false;
    }

    true
}
