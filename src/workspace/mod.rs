//! # Workspace Structure and Rules Manager
//!
//! ## Overview
//! Establishes profile workspace directories, rule file synchronization, and custom skill
//! discovery/mounting across agent environments.
//!
//! ## Submodules
//! - [`paths`] (`paths.rs`): Resolves profile roots, workspace directories, and standard config files.
//! - [`rules`] (`rules.rs`): Selects and merges `AGENTS.md`, `GEMINI.md`, and `CLAUDE.md` rule templates.
//! - [`rules_check`] (`rules_check.rs`): Audits rule synchronization status across workspace targets.
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
pub mod rules_check;
pub mod sync;
pub mod sync_helpers;
pub mod sync_layout;
pub mod skills;
pub mod skills_helpers;

/// Alias module for workspace initialization tests.
pub mod init {
    pub use super::sync::init_workspace;
}

/// Alias module for rules selector tests.
pub mod rules_selector {
    pub use super::rules::{AuthResult, AuthStatus, RulesSelector};
}

/// Alias module for rule sync tests.
pub mod rule_sync {
    pub use super::sync::sync_rule_files;
}

/// Alias module for skill sync tests.
pub mod skill_sync {
    pub use super::skills::{
        cleanup_ductor_links, discover_skills, has_valid_skill_frontmatter, resolve_canonical,
        sync_bundled_skills, sync_skills, watch_skill_sync, SkillConfig,
    };
}
