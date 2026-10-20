//! # Temporary Storage Cleanup Module (index.md)
//!
//! ## Overview
//! Cleans stale files, old history files, and orphaned processes periodically.
//!
//! ## Submodules
//! - [`observer`] (`observer.rs`): Monitors storage usage and purges items exceeding retention limits.
//!
//! ## Search Tags
//! #cleaner, #temp-cleanup, #retention-limits

pub mod observer;
