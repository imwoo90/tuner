//! # Workspace Structure and Rules Manager
//!
//! ## Overview
//! Establishes profile workspace directories, rule file synchronization, and custom skill
//! discovery/mounting across agent environments.
//!
//! ## Submodules
//! - [`paths`] (`paths.rs`): Resolves profile roots, workspace directories, and standard config files.
//! - [`rules`] (`rules.rs`): Selects and deploys `AGENTS.md` rule templates and cleans stale files.
//! - [`sync`] (`sync.rs`): Initializes workspace folder hierarchies and synchronizes rule templates.
//! - [`sync_helpers`] (`sync_helpers.rs`): File hashing, backup, and atomic copy helpers for workspace sync.
//! - [`sync_layout`] (`sync_layout.rs`): Default directory layout definitions and scaffolding generators.
//! - [`skills`] (`skills.rs`): Discovers, validates, and symlinks custom and bundled agent skills.
//! - [`skills_helpers`] (`skills_helpers.rs`): Frontmatter validation and symlink resolution for skills.
//!
//! ## Search Tags
//! #workspace-setup, #rule-selector, #skill-sync, #profile-paths

pub mod paths;
pub mod rules;
pub mod sync;
pub mod sync_helpers;
pub mod sync_layout;
pub mod skills;
pub mod skills_helpers;
