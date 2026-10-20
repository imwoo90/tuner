//! # Status Telemetry & Quiet Hours Module (index.md)
//!
//! ## Overview
//! Sends health telemetry logs and manages quiet hour status constraints.
//!
//! ## Submodules
//! - [`quiet`] (`quiet.rs`): Checks time configurations to enforce quiet hours.
//! - [`scheduler`] (`scheduler.rs`): Ticks and dispatches periodic health heartbeat events.
//!
//! ## Search Tags
//! #heartbeat, #quiet-hours, #health-telemetry

pub mod quiet;
pub mod scheduler;
