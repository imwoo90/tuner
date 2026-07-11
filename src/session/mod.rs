//! # Session Lifecycle Management
//!
//! This module coordinates session data, storage key representations, and JSON-based
//! persistence for messaging interface sessions.

pub mod key;
pub mod data;
pub mod manager;
pub mod freshness;
pub mod named;

#[cfg(test)]
pub mod manager_tests;
#[cfg(test)]
pub mod named_tests;
