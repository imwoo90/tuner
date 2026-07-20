//! # Cron Scheduler & Job Automation Module (index.md)
//!
//! ## Overview
//! Provides a timezone-aware scheduler allowing automatic, periodic runs of AI agents in specified folders.
//!
//! ## Module Components
//! - [`manager`]: Handles listing, saving, enabling/disabling, and persisting scheduled jobs in JSON files.
//! - [`scheduler`]: Runs the ticks, monitors file modification times, checks quiet hours, and spawns tokio threads to execute jobs.
//!
//! ## Data Flow Diagram
//! ```text
//! [ cron_tasks.json ] ──> [ CronManager ] <── [ Telegram commands ]
//!                               │
//!                         (polls file)
//!                               ▼
//!                        [ CronScheduler ]
//!                               │ (ticks every 5s)
//!                               ▼
//!                 [ tokio::spawn(execute_job) ] ──> [ AntigravityCli ]
//! ```
//!
//! ## Search Tags
//! #cron-scheduler, #job-automation, #periodic-runs, #cron-manager, #timezone-evaluation

pub mod manager;
pub mod scheduler;

pub use manager::{CronJob, CronManager};
pub use scheduler::CronScheduler;
