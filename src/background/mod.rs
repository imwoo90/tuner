//! # Background Task Executor Module (index.md)
//!
//! ## Overview
//! Handles fire-and-forget background execution of AI agent provider commands.
//!
//! ## Module Components
//! - [`models`]: Data structures representing task targets and execution parameters.
//! - [`observer`]: Task registry, chat limits, timeout tracking, and process drop guards.
//!
//! ## Search Tags
//! #background-runner, #observer, #async-execution, #task-limits

pub mod models;
pub mod observer;
pub mod test_utils;

pub use models::{BackgroundResult, BackgroundResultStatus, BackgroundSubmit};
pub use observer::BackgroundObserver;
