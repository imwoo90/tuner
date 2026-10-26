//! # Security and Path Safety Module
//!
//! ## Overview
//! Validates filesystem path bounds to prevent directory traversal and filters output strings
//! against credential leakage rules.
//!
//! ## Submodules
//! - [`paths`] (`paths.rs`): Verifies path targets are restricted to authorized workspace directories.
//! - [`vault`] (`vault.rs`): Encrypted per-profile credential store (`XSalsa20-Poly1305`) and session persistence.
//! - [`vault_cli`] (`vault_cli.rs`): Zero-knowledge `--vault` CLI runner with output secret redaction.
//!
//! ## Search Tags
//! #safety-module, #path-sandbox, #credential-vault

pub mod paths;
pub mod vault;
pub mod vault_cli;
