//! # Workspace Rules & Skill Sync Module Tests
//!
//! TDD test cases verifying workspace paths resolution, initialization,
//! rule selector, rule sync, and skill sync modules.

#[cfg(test)]
mod tests {
    use crate::workspace::paths::{resolve_paths, TunerPaths};
    use crate::workspace::rules::RulesSelector;
    use crate::workspace::skills::{discover_skills, sync_skills};
    use crate::workspace::sync::{init_workspace, sync_rule_files};
    use std::fs;
    use std::path::{Path, PathBuf};
    use tempfile::tempdir;

    fn setup_mock_defaults(fw: &Path) {
        let defaults = fw.join("_home_defaults");
        let cfg_dir = defaults.join("config");
        fs::create_dir_all(&cfg_dir).unwrap();
        fs::write(cfg_dir.join("config.json"), r#"{"provider":"antigravity","model":"gemini-3.8-flash"}"#).unwrap();

        let ws = defaults.join("workspace");
        fs::create_dir_all(&ws).unwrap();
        fs::write(ws.join("RULES.md"), "# Tuner Workspace Prompt").unwrap();

        let ms = ws.join("memory_system");
        fs::create_dir_all(&ms).unwrap();
        fs::write(ms.join("RULES.md"), "# Memory System Rules").unwrap();

        let tools = ws.join("tools");
        let media = tools.join("media_tools");
        fs::create_dir_all(&media).unwrap();
        fs::write(media.join("RULES.md"), "# Media Tools Rules").unwrap();
        fs::write(media.join("transcribe_audio.py"), "# audio tool").unwrap();
    }

    #[test]
    fn test_workspace_paths_properties() {
        let home = PathBuf::from("/home/user/.tuner");
        let fw = PathBuf::from("/opt/tuner");
        let hd = fw.join("_home_defaults");
        let paths = TunerPaths::new(home, hd, None);

        assert_eq!(paths.workspace(), PathBuf::from("/home/user/.tuner/workspace"));
        assert_eq!(paths.config_path(), PathBuf::from("/home/user/.tuner/config/config.json"));
        assert_eq!(paths.cron_jobs_path(), PathBuf::from("/home/user/.tuner/cron_jobs.json"));
    }

    #[test]
    fn test_workspace_paths_properties_with_profile() {
        let home = PathBuf::from("/home/user/.tuner");
        let fw = PathBuf::from("/opt/tuner");
        let hd = fw.join("_home_defaults");
        let paths = TunerPaths::new(home, hd, Some("my-profile".to_string()));

        assert_eq!(paths.workspace(), PathBuf::from("/home/user/.tuner/profiles/my-profile/workspace"));
        assert_eq!(paths.sessions_path(), PathBuf::from("/home/user/.tuner/profiles/my-profile/sessions.json"));
        assert_eq!(paths.config_path(), PathBuf::from("/home/user/.tuner/config/config.json"));
    }

    #[test]
    fn test_resolve_paths_explicit() {
        let paths = resolve_paths(
            Some(PathBuf::from("/h")),
            Some(PathBuf::from("/f")),
            Some(PathBuf::from("/d")),
            Some("default".to_string()),
        );
        assert_eq!(paths.tuner_home, PathBuf::from("/h"));
        assert_eq!(paths.profile.as_deref(), Some("default"));
    }

    #[test]
    fn test_init_creates_directories_and_copies_files() {
        let tmp = tempdir().unwrap();
        let fw = tmp.path().join("fw");
        setup_mock_defaults(&fw);

        let paths = TunerPaths::new(
            tmp.path().join("home"),
            fw.join("_home_defaults"),
            Some("default".to_string()),
        );

        init_workspace(&paths).unwrap();
        assert!(paths.workspace().join("AGENTS.md").is_file());
        assert!(paths.workspace().join(".agents").join("rules").join("constraints.md").is_file());
    }

    #[test]
    fn test_sync_rule_files_migrates_mainmemory_and_protects_projects() {
        let tmp = tempdir().unwrap();
        let ws = tmp.path();

        let proj_dir = ws.join("projects").join("rust-agent-native-template");
        let ms_dir = ws.join("memory_system");
        fs::create_dir_all(&proj_dir).unwrap();
        fs::create_dir_all(&ms_dir).unwrap();

        fs::write(proj_dir.join("AGENTS.md"), "proj agents original").unwrap();
        fs::write(proj_dir.join("GEMINI.md"), "proj gemini pointer").unwrap();
        fs::write(ms_dir.join("MAINMEMORY.md"), "# User Facts").unwrap();

        sync_rule_files(ws).unwrap();

        assert_eq!(fs::read_to_string(proj_dir.join("AGENTS.md")).unwrap(), "proj agents original");
        assert_eq!(fs::read_to_string(proj_dir.join("GEMINI.md")).unwrap(), "proj gemini pointer");
        assert!(ws.join(".agents").join("rules").join("mainmemory.md").is_file());
        assert!(ws.join(".agents").join("rules").join("constraints.md").is_file());
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
    fn test_sync_skills_creates_symlinks_and_prunes_cross_profile_leaks() {
        let tmp = tempdir().unwrap();
        let home = tmp.path().join("home");
        let paths = TunerPaths::new(
            home.clone(),
            tmp.path().join("fw").join("_home_defaults"),
            Some("default".to_string()),
        );
        let skill_dir = paths.skills_dir().join("custom_skill");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: custom_skill\ndescription: profile skill\n---\n# Body",
        ).unwrap();

        let other_prof_skill = home.join("profiles").join("other").join("workspace").join("skills").join("leaked_skill");
        fs::create_dir_all(&other_prof_skill).unwrap();
        fs::write(
            other_prof_skill.join("SKILL.md"),
            "---\nname: leaked_skill\ndescription: leaked\n---\n# Body",
        ).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&other_prof_skill, paths.skills_dir().join("leaked_skill")).unwrap();

        sync_skills(&paths).unwrap();

        assert!(paths.agents_skills_dir().join("custom_skill").exists());
        assert!(!paths.skills_dir().join("leaked_skill").exists());
        assert!(!paths.agents_skills_dir().join("leaked_skill").exists());

        let archive_dir = paths.skills_dir().join("archive");
        fs::create_dir_all(&archive_dir).unwrap();
        fs::rename(&skill_dir, archive_dir.join("custom_skill")).unwrap();
        sync_skills(&paths).unwrap();
        assert!(!paths.agents_skills_dir().join("custom_skill").exists());
        assert!(archive_dir.join("custom_skill").join("SKILL.md").is_file());
    }

    #[test]
    fn test_deploy_rules_deploys_agents_and_cleans_up_legacy_files() {
        let tmp = tempdir().unwrap();
        let home = tmp.path().join("home");
        let fw = tmp.path().join("fw");
        let defaults = fw.join("_home_defaults");
        let ws = defaults.join("workspace");
        fs::create_dir_all(&ws).unwrap();
        fs::write(ws.join("RULES.md"), "# Base Rules").unwrap();

        let paths = TunerPaths::new(
            home,
            defaults,
            Some("default".to_string()),
        );

        let profile_ws = paths.workspace();
        let out_dir = profile_ws.join("output_to_user");
        let tools_dir = profile_ws.join("tools").join("cron_tools");
        fs::create_dir_all(&out_dir).unwrap();
        fs::create_dir_all(&tools_dir).unwrap();
        fs::write(profile_ws.join("GEMINI.md"), "# Old Gemini").unwrap();
        fs::write(out_dir.join("AGENTS.md"), "# Legacy output_to_user AGENTS").unwrap();
        fs::write(tools_dir.join("AGENTS.md"), "# Legacy cron_tools AGENTS").unwrap();

        let selector = RulesSelector::new(paths);
        selector.deploy_rules().unwrap();

        assert!(profile_ws.join("AGENTS.md").is_file());
        assert!(!profile_ws.join("GEMINI.md").exists());
        assert!(!out_dir.join("AGENTS.md").exists());
        assert!(!tools_dir.join("AGENTS.md").exists());
    }

    #[test]
    fn test_sync_constraints_rule_creates_and_preserves() {
        use crate::workspace::sync_helpers::sync_constraints_rule;
        let tmp = tempdir().unwrap();
        let ws = tmp.path().join("workspace");
        let def_tpl = tmp.path().join("default_constraints.md");
        fs::write(&def_tpl, "---\ntrigger: always_on\n---\n# Defaults").unwrap();

        sync_constraints_rule(&ws, Some(&def_tpl)).unwrap();
        let target = ws.join(".agents").join("rules").join("constraints.md");
        assert!(target.is_file());
        let content1 = fs::read_to_string(&target).unwrap();
        assert!(content1.contains("# Defaults"));

        fs::write(&target, "---\ntrigger: always_on\n---\n# My Custom Constraints").unwrap();

        sync_constraints_rule(&ws, Some(&def_tpl)).unwrap();
        let content2 = fs::read_to_string(&target).unwrap();
        assert!(content2.contains("# My Custom Constraints"));
    }
}
