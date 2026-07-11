//! # Workspace Module
//!
//! Provides features for workspace paths, initialization, rules selection/sync,
//! and skill directory synchronization.

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
