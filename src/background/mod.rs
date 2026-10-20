//! # Background Task Executor Module (index.md)
//!
//! ## Overview
//! Handles fire-and-forget background execution of AI agent provider commands.
//!
//! ## Submodules
//! - [`models`] (`models.rs`): Data structures representing task targets and execution parameters.
//! - [`observer`] (`observer.rs`): Task registry, chat limits, timeout tracking, and process drop guards.
//! - [`test_utils`] (`test_utils.rs`): Mock providers and helper builders for background task testing.
//!
//! ## Search Tags
//! #background-runner, #observer, #async-execution, #task-limits

pub mod models;
pub mod observer;
pub mod test_utils;

pub use models::{BackgroundResult, BackgroundResultStatus, BackgroundSubmit};
pub use observer::BackgroundObserver;
