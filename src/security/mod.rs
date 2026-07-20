//! # Security and Path Safety Module (index.md)
//!
//! ## Overview
//! Validates path bounds to prevent directory traversal and checks output strings against safety rules.
//!
//! ## Module Components
//! - [`paths`]: Verifies path targets are restricted to authorized directories.
//! - [`content`]: Filters output text to block credential leakage.
//!
//! ## Search Tags
//! #safety-module, #path-sandbox, #content-filter

pub mod paths;
pub mod content;
