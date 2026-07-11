//! # Background Runner Module
//!
//! This module manages fire-and-forget background execution of CLI agent tasks.

pub mod models;
pub mod observer;
pub mod test_utils;

pub use models::{BackgroundResult, BackgroundResultStatus, BackgroundSubmit};
pub use observer::BackgroundObserver;
