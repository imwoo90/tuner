//! # Cron Scheduler & Job Automation Module (index.md)
//!
//! ## Overview
//! Provides a timezone-aware scheduler allowing automatic, periodic runs of AI agents in specified folders.
//!
//! ## Submodules
//! - [`manager`] (`manager.rs`): Handles listing, saving, enabling/disabling, and persisting scheduled jobs in JSON files.
//! - [`scheduler`] (`scheduler.rs`): Runs the ticks, monitors file modification times, checks quiet hours, and spawns tokio tasks.
//!
//! ## Search Tags
//! #cron-scheduler, #job-automation, #periodic-runs, #cron-manager, #timezone-evaluation

pub mod manager;
pub mod scheduler;

pub use manager::{CronJob, CronManager};
pub use scheduler::CronScheduler;
