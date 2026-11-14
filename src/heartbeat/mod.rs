//! # Status Telemetry & Quiet Hours Module (index.md)
//!
//! ## Overview
//! Sends health telemetry logs and manages quiet hour status constraints.
//!
//! ## Submodules
//! - [`scheduler`] (`scheduler.rs`): Ticks and dispatches periodic health heartbeat events.
//!
//! ## Search Tags
//! #heartbeat, #quiet-hours, #health-telemetry

use chrono::{DateTime, NaiveTime, Timelike};
use chrono_tz::Tz;

pub mod scheduler;

/// Determine whether the local time of the given timezone is within the quiet hours window.
pub fn is_within_quiet_hours(
    now: &DateTime<Tz>,
    quiet_start: NaiveTime,
    quiet_end: NaiveTime,
) -> bool {
    if quiet_start == quiet_end {
        return false; // same start/end means never quiet
    }

    let current_local_time = NaiveTime::from_hms_opt(
        now.hour(),
        now.minute(),
        now.second(),
    )
    .unwrap_or_default();

    if quiet_start < quiet_end {
        current_local_time >= quiet_start && current_local_time < quiet_end
    } else {
        current_local_time >= quiet_start || current_local_time < quiet_end
    }
}

/// Check if the agent's heartbeat response is equivalent to the acknowledge token.
pub fn should_suppress_heartbeat(output: &str, ack_token: &str) -> bool {
    let clean_output = output.trim();
    let clean_ack = ack_token.trim();
    clean_output == clean_ack
}

/// Timezone-safe quiet windows and ACK suppression re-exports.
pub mod quiet {
    pub use super::{is_within_quiet_hours, should_suppress_heartbeat};
}

pub(crate) fn should_deliver_heartbeat_response(
    chat_id: i64,
    topic_id: Option<i64>,
    start_sid: &str,
    ack_token: &str,
    resp: &crate::cli::CliResponse,
) -> bool {
    if resp.is_error {
        eprintln!(
            "⚠️ [tuner] Heartbeat CLI error for chat {} topic {:?}: {}",
            chat_id, topic_id, resp.stderr
        );
        return false;
    }
    if !start_sid.is_empty()
        && resp
            .session_id
            .as_deref()
            .is_some_and(|s| !s.is_empty() && s != start_sid)
    {
        eprintln!(
            "⚠️ [tuner] Heartbeat ignored mismatched session_id for chat {} topic {:?}",
            chat_id, topic_id
        );
        return false;
    }
    !resp.result.trim().is_empty() && !should_suppress_heartbeat(&resp.result, ack_token)
}

pub(crate) fn reconcile_heartbeat_session(
    latest: &mut crate::session::data::SessionData,
    prov_key: &str,
    start_sid: &str,
    resp_sid: Option<&str>,
    active: &mut bool,
) -> bool {
    let latest_sid = latest.get_session_id(prov_key);
    let was_reset = (!start_sid.is_empty()
        && (latest.get_past_session_ids(prov_key).iter().any(|p| p == start_sid)
            || latest_sid != start_sid))
        || (start_sid.is_empty() && !latest_sid.is_empty() && Some(latest_sid.as_str()) != resp_sid);
    if was_reset {
        return false;
    }
    *active = true;
    if start_sid.is_empty()
        && let Some(new_sid) = resp_sid.filter(|s| !s.is_empty())
        && latest_sid != new_sid
    {
        latest.set_session_id(prov_key, new_sid);
        return true;
    }
    false
}

pub(crate) fn build_heartbeat_cli_config(
    base_cfg: &crate::config::CliConfig,
    session: &crate::session::data::SessionData,
) -> crate::config::CliConfig {
    let mut cfg = base_cfg.clone();
    cfg.chat_id = session.chat_id;
    cfg.topic_id = session.topic_id;
    session.apply_to_cli_config(&mut cfg);
    cfg
}
