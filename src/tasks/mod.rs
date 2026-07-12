//! # Tasks Module
//!
//! Handles background task registration, persistent state tracking,
//! prioritization/scheduler execution constraints, and host run coordination.

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
