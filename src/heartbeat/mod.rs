//! # Status Telemetry & Quiet Hours Module (index.md)
//!
//! ## Overview
//! Sends health telemetry logs and manages quiet hour status constraints.
//!
//! ## Module Components
//! - [`scheduler`]: Ticks and sends health logs.
//! - [`quiet`]: Checks time configurations to enforce quiet hours.
//!
//! ## Search Tags
//! #heartbeat, #quiet-hours, #health-telemetry

pub mod quiet;
pub mod scheduler;
