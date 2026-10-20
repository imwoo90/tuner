//! # Session Lifecycle Management
//!
//! ## Overview
//! Manages conversation histories, workspace session initializations, and session key resolution
//! for active user and group chat threads.
//!
//! ## Submodules
//! - [`key`] (`key.rs`): Unique session identifiers matching transport, chat, and forum topic mappings.
//! - [`data`] (`data.rs`): In-memory and JSON-serialized structure of active session state.
//! - [`manager`] (`manager.rs`): Resolves, loads, updates, and resets persisted sessions.
//! - [`named`] (`named.rs`): Maintains human-readable session alias mappings for background follow-ups.
//!
//! ## Search Tags
//! #session-lifecycle, #session-manager, #timezone-reset, #alias-resolver

use chrono::{DateTime, Duration, LocalResult, TimeZone, Utc};
use chrono_tz::Tz;

pub mod key;
pub mod data;
pub mod manager;
pub mod named;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
pub mod manager_tests;
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
/// Test module/item.
pub mod named_tests;
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
pub mod topic_history_tests;

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
        &now_local.date_naive().and_hms_opt(daily_reset_hour % 24, 0, 0).unwrap_or_default(),
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
    session: &data::SessionData,
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

    if idle_timeout_minutes > 0
        && now.signed_duration_since(last) >= Duration::minutes(idle_timeout_minutes)
    {
        return false;
    }

    if daily_reset_enabled
        && has_crossed_daily_reset(&last, &now, user_timezone, daily_reset_hour)
    {
        return false;
    }

    true
}

/// Session freshness evaluation re-exports.
pub mod freshness {
    pub use super::{has_crossed_daily_reset, is_session_fresh};
}
