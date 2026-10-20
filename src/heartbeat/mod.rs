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
