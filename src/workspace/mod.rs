//! # Workspace Structure and Rules Manager (index.md)
//!
//! ## Overview
//! Establishes workspace directories, rule settings, and custom skills synchronizers.
//!
//! ## Module Components
//! - [`paths`]: Translates profile configs to workspace directories.
//! - [`rules`]: Resolves GEMINI.md/CLAUDE.md/AGENTS.md rule templates.
//! - [`sync`]: Initializes folders and clones rule parameters.
//! - [`skills`]: Scans and mounts custom skills.
//!
//! ## Search Tags
//! #workspace-setup, #rule-selector, #skill-sync, #profile-paths

pub mod paths;
pub mod rules;
pub mod rules_check;
pub mod sync;
pub mod sync_helpers;
pub mod skills;
pub mod skills_helpers;

/// Alias module for workspace initialization tests.
pub mod init {
    pub use super::sync::{init_workspace, inject_runtime_environment};
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
