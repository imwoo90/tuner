//! # Session Lifecycle Management
//!
//! ## Overview
//! Manages conversation histories, workspace session initializations, and session key resolution
//! for active user and group chat threads.
//!
//! ## Submodules
//! - [`key`] (`key.rs`): Unique session identifiers matching transport, chat, and forum topic mappings.
//! - [`data`] (`data.rs`): In-memory and JSON-serialized structure of active session state.
//! - [`manager`] (`manager.rs`): Resolves, loads, updates, and resets persisted sessions.
//! - [`freshness`] (`freshness.rs`): Inspects session age, inactivity timeouts, and daily reset policies.
//! - [`named`] (`named.rs`): Maintains human-readable session alias mappings for background follow-ups.
//!
//! ## Search Tags
//! #session-lifecycle, #session-manager, #timezone-reset, #alias-resolver

pub mod key;
pub mod data;
pub mod manager;
pub mod freshness;
pub mod named;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
pub mod manager_tests;
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
/// Test module/item.
pub mod named_tests;
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
pub mod topic_history_tests;
