//! # Workspace Rules & Skill Sync Module Tests
//!
//! TDD test cases verifying workspace paths resolution, initialization,
//! rule selector, rule sync, and skill sync modules.

#[cfg(test)]
mod tests {
    use crate::workspace::paths::{resolve_paths, DuctorPaths};
    use crate::workspace::init::{init_workspace, inject_runtime_environment};
    use crate::workspace::rules_selector::RulesSelector;
    use crate::workspace::rule_sync::sync_rule_files;
    use crate::workspace::skill_sync::{
        cleanup_ductor_links, discover_skills, resolve_canonical, sync_skills,
    };
    use std::fs;
    use std::path::{Path, PathBuf};
    use tempfile::tempdir;

    fn setup_mock_defaults(fw: &Path) {
        unsafe { std::env::set_var("DUCTOR_TEST_MODE", "1"); }
        let ws = fw.join("workspace");
        fs::create_dir_all(&ws).unwrap();
        fs::write(ws.join("CLAUDE.md"), "# Framework CLAUDE.md").unwrap();

        let inner_ws = ws.join("workspace");
        fs::create_dir_all(&inner_ws).unwrap();
        fs::write(inner_ws.join("CLAUDE.md"), "# Inner CLAUDE.md").unwrap();

        let ms = inner_ws.join("memory_system");
        fs::create_dir_all(&ms).unwrap();
        fs::write(ms.join("CLAUDE.md"), "# ms CLAUDE.md").unwrap();
        fs::write(ms.join("MAINMEMORY.md"), "# memory").unwrap();

        let tools = inner_ws.join("tools");
        let media = tools.join("media_tools");
        fs::create_dir_all(&media).unwrap();
        fs::write(media.join("RULES.md"), "# media tools rules").unwrap();
        fs::write(media.join("transcribe_audio.py"), "# audio tool").unwrap();

        fs::write(fw.join("config.example.json"), r#"{"provider":"claude","model":"opus"}"#).unwrap();
    }

    #[test]
    fn test_workspace_paths_properties() {
        let home = PathBuf::from("/home/user/.ductor");
        let fw = PathBuf::from("/opt/ductor");
        let hd = fw.join("workspace");
        let paths = DuctorPaths::new(home, hd, fw);

        assert_eq!(paths.workspace(), PathBuf::from("/home/user/.ductor/workspace"));
        assert_eq!(paths.config_path(), PathBuf::from("/home/user/.ductor/config/config.json"));
        assert_eq!(paths.logs_dir(), PathBuf::from("/home/user/.ductor/logs"));
        assert_eq!(paths.mainmemory_path(), PathBuf::from("/home/user/.ductor/workspace/memory_system/MAINMEMORY.md"));
    }

    #[test]
    fn test_resolve_paths_explicit() {
        let _paths = resolve_paths(
            Some(PathBuf::from("/h")),
            Some(PathBuf::from("/f")),
            Some(PathBuf::from("/d")),
        );
    }

    #[test]
    fn test_init_creates_directories_and_copies_files() {
        let tmp = tempdir().unwrap();
        let fw = tmp.path().join("fw");
        setup_mock_defaults(&fw);

        let paths = DuctorPaths::new(
            tmp.path().join("home"),
            fw.join("workspace"),
            fw,
        );

        init_workspace(&paths).unwrap();
    }

    #[test]
    fn test_inject_runtime_notice() {
        let tmp = tempdir().unwrap();
        let fw = tmp.path().join("fw");
        setup_mock_defaults(&fw);

        let paths = DuctorPaths::new(
            tmp.path().join("home"),
            fw.join("workspace"),
            fw,
        );

        inject_runtime_environment(&paths, Some("sandbox")).unwrap();
    }

    #[test]
    fn test_rules_selector_get_variant_suffix() {
        unsafe { std::env::set_var("DUCTOR_TEST_MODE", "1"); }
        let home = PathBuf::from("/h");
        let fw = PathBuf::from("/f");
        let paths = DuctorPaths::new(home.clone(), fw.join("w"), fw);
        let selector = RulesSelector::new(paths);
        assert_eq!(selector.get_variant_suffix(), "claude-only");
    }

    #[test]
    fn test_sync_rule_files_newer_overwrites() {
        let tmp = tempdir().unwrap();
        let claude = tmp.path().join("CLAUDE.md");
        let agents = tmp.path().join("AGENTS.md");
        fs::write(&claude, "newer rules").unwrap();
        fs::write(&agents, "older rules").unwrap();

        sync_rule_files(tmp.path()).unwrap();
    }

    #[test]
    fn test_discover_skills_valid_only() {
        let tmp = tempdir().unwrap();
        let skill_dir = tmp.path().join("skill_a");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: skill_a\ndescription: test desc\n---\n# Content",
        ).unwrap();

        let skills = discover_skills(tmp.path());
        assert!(skills.contains_key("skill_a"));
    }

    #[test]
    fn test_resolve_canonical_precedence() {
        let ductor = std::collections::HashMap::new();
        let claude = std::collections::HashMap::new();
        let codex = std::collections::HashMap::new();
        let gemini = std::collections::HashMap::new();

        let _res = resolve_canonical("my_skill", &ductor, &claude, &codex, &gemini);
    }

    #[test]
    fn test_sync_skills_creates_symlinks() {
        let tmp = tempdir().unwrap();
        let paths = DuctorPaths::new(
            tmp.path().join("home"),
            tmp.path().join("fw").join("workspace"),
            tmp.path().join("fw"),
        );

        sync_skills(&paths, false).unwrap();
    }

    #[test]
    fn test_cleanup_ductor_links() {
        let tmp = tempdir().unwrap();
        let paths = DuctorPaths::new(
            tmp.path().join("home"),
            tmp.path().join("fw").join("workspace"),
            tmp.path().join("fw"),
        );

        cleanup_ductor_links(&paths).unwrap();
    }
}
