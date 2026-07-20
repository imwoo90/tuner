//! # Session Lifecycle Management (index.md)
//!
//! ## Overview
//! Manages message histories, workspace initializations, and key resolution for active user/group chat threads.
//!
//! ## Module Components
//! - [`key`]: Represents unique session identifiers matching chat/topic mappings.
//! - [`data`]: In-memory and serialized structure of active session data.
//! - [`manager`]: Resolves, loads, updates, and resets sessions.
//! - [`freshness`]: Inspects session age, inactivity timeouts, and triggers daily resets.
//! - [`named`]: Maintains session-to-alias maps.
//!
//! ## Search Tags
//! #session-lifecycle, #session-manager, #timezone-reset, #alias-resolver

pub mod key;
pub mod data;
pub mod manager;
pub mod freshness;
pub mod named;

#[cfg(test)]
pub mod manager_tests;
#[cfg(test)]
pub mod named_tests;
