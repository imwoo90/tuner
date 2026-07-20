//! # DAG Automation Tasks Module (index.md)
//!
//! ## Overview
//! Manages task registries, task tracking databases, execution priorities, and runs dependency DAGs.
//!
//! ## Module Components
//! - [`registry`]: Registers and loads executable automation tasks.
//! - [`engine`]: Runs dependency DAG trees in execution sequence.
//! - [`runner`]: Runs process executors and tracks process PIDs.
//! - [`hub`]: Orchestrates callback channels for tasks.
//!
//! ## Search Tags
//! #dag-scheduler, #task-registry, #execution-engine, #dag-runner

pub mod models;
pub mod registry;
pub mod runner;
pub mod engine;
pub mod cleanup;
pub mod dag;
pub mod manager;
pub mod hub;

pub use models::{TaskEntry, TaskSubmit, TaskResult, TasksConfig};
pub use registry::TaskRegistry;
pub use runner::ProcessRegistry;
pub use hub::{TaskHub, TaskResultCallback, QuestionHandler, TaskInFlight};
