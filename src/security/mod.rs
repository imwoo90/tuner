//! # Security and Path Safety Module
//!
//! ## Overview
//! Validates filesystem path bounds to prevent directory traversal and filters output strings
//! against credential leakage rules.
//!
//! ## Submodules
//! - [`paths`] (`paths.rs`): Verifies path targets are restricted to authorized workspace directories.
//! - [`content`] (`content.rs`): Filters output text to redact credentials and sensitive tokens.
//!
//! ## Search Tags
//! #safety-module, #path-sandbox, #content-filter

pub mod paths;
pub mod content;
