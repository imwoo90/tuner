//! # Timezone-safe Quiet Windows and ACK Suppression
//!
//! This module implements local timezone calculations for quiet hours and
//! parsing logic for suppressing empty heartbeat alert tokens.

//! 
//! ## Search Tags
//! #quiet

use chrono::{NaiveTime, DateTime, Timelike};
use chrono_tz::Tz;

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
    ).unwrap();

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
