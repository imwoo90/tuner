use super::*;
use crate::config::CliConfig;

fn create_test_dir() -> tempfile::TempDir {
    tempfile::Builder::new().prefix("tuner_mod_").tempdir().unwrap()
}

#[test]
fn test_antigravity_command_uses_print_and_conversation() {
    let _guard = crate::telegram::TEST_ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let temp_dir = create_test_dir();
    let config = CliConfig {
        provider: "antigravity".to_string(),
        model: Some("antigravity-default".to_string()),
        working_dir: temp_dir.path().to_path_buf(),
        ..Default::default()
    };
    let cli = AntigravityCli::new(config);
    let cmd = cli.build_command("hi there", Some("conv-1"), false);

    assert_eq!(cmd[0], "agy");
    assert_eq!(cmd.iter().filter(|&s| s == "--model").count(), 0);
    assert!(cmd.contains(&"--conversation".to_string()));
    assert!(cmd.contains(&"conv-1".to_string()));
    assert_eq!(cmd[cmd.len() - 2..], ["--print".to_string(), "hi there".to_string()]);
}

#[test]
fn test_antigravity_command_grounds_in_workspace() {
    let _guard = crate::telegram::TEST_ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let temp_dir = create_test_dir();
    let config = CliConfig {
        provider: "antigravity".to_string(),
        working_dir: temp_dir.path().to_path_buf(),
        ..Default::default()
    };
    let cli = AntigravityCli::new(config);
    let cmd = cli.build_command("hi", None, false);

    assert!(cmd.contains(&"--add-dir".to_string()));
    let idx = cmd.iter().position(|s| s == "--add-dir").unwrap();
    assert_eq!(cmd[idx + 1], cli.agy_workspace().to_string_lossy().to_string());
}

#[test]
fn test_antigravity_command_includes_selected_model() {
    let _guard = crate::telegram::TEST_ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let temp_dir = create_test_dir();
    let config = CliConfig {
        provider: "antigravity".to_string(),
        model: Some("claude-sonnet-5-5".to_string()),
        working_dir: temp_dir.path().to_path_buf(),
        ..Default::default()
    };
    let cli = AntigravityCli::new(config);
    let cmd = cli.build_command("hi", None, false);

    assert!(cmd.contains(&"--model".to_string()));
    let model_idx = cmd.iter().position(|s| s == "--model").unwrap();
    assert_eq!(cmd[model_idx + 1], "claude-sonnet-5-5");
    
    let print_idx = cmd.iter().position(|s| s == "--print").unwrap();
    assert!(model_idx < print_idx);
}

#[test]
fn test_antigravity_command_continue_and_bypass() {
    let _guard = crate::telegram::TEST_ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let temp_dir = create_test_dir();
    let config = CliConfig {
        provider: "antigravity".to_string(),
        permission_mode: "bypassPermissions".to_string(),
        working_dir: temp_dir.path().to_path_buf(),
        ..Default::default()
    };
    let cli = AntigravityCli::new(config);
    let cmd = cli.build_command("hi", None, true);

    assert!(cmd.contains(&"--continue".to_string()));
    assert!(cmd.contains(&"--dangerously-skip-permissions".to_string()));
    assert_eq!(cmd[cmd.len() - 2..], ["--print".to_string(), "hi".to_string()]);
}

#[test]
fn test_antigravity_command_includes_cli_parameters() {
    let _guard = crate::telegram::TEST_ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let temp_dir = create_test_dir();
    let mut cli_params = std::collections::HashMap::new();
    cli_params.insert("antigravity".to_string(), vec!["--log-file".to_string(), "agy.log".to_string()]);
    let config = CliConfig {
        provider: "antigravity".to_string(),
        cli_parameters: cli_params,
        working_dir: temp_dir.path().to_path_buf(),
        ..Default::default()
    };
    let cli = AntigravityCli::new(config);
    let cmd = cli.build_command("hi", None, false);

    let log_idx = cmd.iter().position(|s| s == "--log-file").unwrap();
    assert_eq!(cmd[log_idx + 1], "agy.log");
    
    let print_idx = cmd.iter().position(|s| s == "--print").unwrap();
    assert!(log_idx < print_idx);
    assert_eq!(cmd[cmd.len() - 2..], ["--print".to_string(), "hi".to_string()]);
}

#[test]
fn test_antigravity_dotted_workspace_mapped_to_symlink() {
    let _guard = crate::telegram::TEST_ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let temp_dir = create_test_dir();
    let base = temp_dir.path().join(".tuner").join("workspace");
    std::fs::create_dir_all(&base).unwrap();

    let config = CliConfig {
        provider: "antigravity".to_string(),
        working_dir: base.clone(),
        ..Default::default()
    };
    let cli = AntigravityCli::new(config);
    let agy_ws = cli.agy_workspace();

    let agy_ws_str = agy_ws.to_string_lossy();
    assert!(!agy_ws_str.contains("/."));
    assert!(agy_ws_str.ends_with("/tuner/workspace"));
    assert_eq!(agy_ws.canonicalize().unwrap(), base.canonicalize().unwrap());

    let cmd = cli.build_command("hi", None, false);
    let idx = cmd.iter().position(|s| s == "--add-dir").unwrap();
    assert_eq!(cmd[idx + 1], agy_ws.to_string_lossy().to_string());
}

#[test]
fn test_antigravity_plain_workspace_unchanged() {
    let plain = PathBuf::from("/home/user/projects/plain_test");

    let config = CliConfig {
        provider: "antigravity".to_string(),
        working_dir: plain.clone(),
        ..Default::default()
    };
    let cli = AntigravityCli::new(config);
    
    assert_eq!(cli.agy_workspace(), plain);
}

#[test]
fn test_format_prompt_injects_system_prompts_without_duplicating_native_rules() {
    let temp_dir = create_test_dir();
    std::fs::write(temp_dir.path().join("AGENTS.md"), "NATIVE_AGENTS_RULES").unwrap();

    let config = CliConfig {
        provider: "antigravity".to_string(),
        working_dir: temp_dir.path().to_path_buf(),
        system_prompt: Some("CUSTOM_SYSTEM".to_string()),
        append_system_prompt: Some("CUSTOM_APPEND".to_string()),
        ..Default::default()
    };
    let cli = AntigravityCli::new(config);
    let final_prompt = cli.format_prompt("user_prompt");

    assert!(final_prompt.contains("CUSTOM_SYSTEM"));
    assert!(final_prompt.contains("CUSTOM_APPEND"));
    assert!(final_prompt.contains("user_prompt"));
    assert!(!final_prompt.contains("NATIVE_AGENTS_RULES"));
}

#[test]
fn test_resolve_cli_model_effort_argon_defaults_to_high() {
    let (m, eff) = AntigravityCli::resolve_cli_model_effort(Some("gemini-4-argon"), None);
    assert_eq!(m.as_deref(), Some("gemini-4-argon"));
    assert_eq!(eff.as_deref(), Some("high"));

    let (m2, eff2) = AntigravityCli::resolve_cli_model_effort(Some("gemini-4-argon"), Some("low"));
    assert_eq!(m2.as_deref(), Some("gemini-4-argon"));
    assert_eq!(eff2.as_deref(), Some("low"));

    let (m3, eff3) = AntigravityCli::resolve_cli_model_effort(Some("gemini-3.1-pro-high"), None);
    assert_eq!(m3.as_deref(), Some("gemini-4-argon"));
    assert_eq!(eff3.as_deref(), Some("high"));

    let (m4, eff4) = AntigravityCli::resolve_cli_model_effort(Some("gemini-3.6-flash"), Some("high"));
    assert_eq!(m4.as_deref(), Some("gemini-4-hydrogen"));
    assert_eq!(eff4.as_deref(), Some("high"));

    let (m5, eff5) = AntigravityCli::resolve_cli_model_effort(Some("flash"), None);
    assert_eq!(m5.as_deref(), Some("gemini-4-hydrogen"));
    assert_eq!(eff5.as_deref(), Some("high"));
}

#[test]
fn test_orchestrator_models_suppress_effort_even_when_forced() {
    let _guard = crate::telegram::TEST_ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    for raw in [
        "gemini-pro-orchestrator",
        "gemini-pro-orchestrator-high",
        "orchestrator",
        "orchestrator-carbon",
    ] {
        let (m, eff) = AntigravityCli::resolve_cli_model_effort(Some(raw), Some("high"));
        assert_eq!(m.as_deref(), Some("gemini-pro-orchestrator"));
        assert_eq!(eff, None);
        assert!(!AntigravityCli::supports_effort(raw));
    }

    for raw in ["gemini-pro-orchestrator-argon", "orchestrator-argon"] {
        let (m, eff) = AntigravityCli::resolve_cli_model_effort(Some(raw), Some("high"));
        assert_eq!(m.as_deref(), Some("gemini-pro-orchestrator-argon"));
        assert_eq!(eff, None);
        assert!(!AntigravityCli::supports_effort(raw));
    }

    let temp_dir = create_test_dir();
    let config = CliConfig {
        provider: "antigravity".to_string(),
        model: Some("gemini-pro-orchestrator".to_string()),
        effort: Some("high".to_string()),
        working_dir: temp_dir.path().to_path_buf(),
        ..Default::default()
    };
    let cli = AntigravityCli::new(config);
    let cmd = cli.build_command("ping", None, false);
    assert!(cmd.contains(&"gemini-pro-orchestrator".to_string()));
    assert!(!cmd.contains(&"--effort".to_string()));
}

#[test]
fn test_hydrogen_xhigh_gpt_oss_medium_and_dynamic_catalog() {
    let (h1, e1) = AntigravityCli::resolve_cli_model_effort(Some("gemini-4-hydrogen-xhigh"), None);
    assert_eq!(h1.as_deref(), Some("gemini-4-hydrogen"));
    assert_eq!(e1.as_deref(), Some("xhigh"));

    let (h2, e2) = AntigravityCli::resolve_cli_model_effort(Some("gemini-4-hydrogen"), Some("max"));
    assert_eq!(h2.as_deref(), Some("gemini-4-hydrogen"));
    assert_eq!(e2.as_deref(), Some("xhigh"));

    let (c1, ec1) = AntigravityCli::resolve_cli_model_effort(Some("gemini-4-carbon"), Some("xhigh"));
    assert_eq!(c1.as_deref(), Some("gemini-4-carbon"));
    assert_eq!(ec1.as_deref(), Some("high"));

    let (g1, eg1) = AntigravityCli::resolve_cli_model_effort(Some("gpt-oss-120b"), Some("high"));
    assert_eq!(g1.as_deref(), Some("gpt-oss-120b-medium"));
    assert_eq!(eg1, None);
    assert!(!AntigravityCli::supports_effort("gpt-oss-120b-medium"));

    let (g2, eg2) = AntigravityCli::resolve_cli_model_effort(Some("gpt-oss-120b-medium"), Some("high"));
    assert_eq!(g2.as_deref(), Some("gpt-oss-120b-medium"));
    assert_eq!(eg2, None);

    let cat = vec![
        "gemini-5-conductor".to_string(),
        "future-oss-low".to_string(),
        "gemini-4-carbon-high".to_string(),
        "gemini-4-carbon-low".to_string(),
    ];
    let (dyn_m, dyn_e) = model_spec::resolve_cli_model_effort_with_catalog(
        Some("gemini-5-conductor"),
        Some("high"),
        Some(&cat),
    );
    assert_eq!(dyn_m.as_deref(), Some("gemini-5-conductor"));
    assert_eq!(dyn_e, None);

    let (single_m, single_e) = model_spec::resolve_cli_model_effort_with_catalog(
        Some("future-oss-low"),
        Some("high"),
        Some(&cat),
    );
    assert_eq!(single_m.as_deref(), Some("future-oss-low"));
    assert_eq!(single_e, None);
}

#[test]
fn test_clamp_invalid_effort_and_format_pty_model_target() {
    assert_eq!(
        model_spec::clamp_effort_for_model("antigravity-default", "max"),
        None
    );
    assert!(!AntigravityCli::supports_effort("antigravity-default"));
    assert_eq!(
        model_spec::clamp_effort_for_model("gemini-4-argon", "turbo").as_deref(),
        Some("high")
    );
    assert_eq!(
        model_spec::clamp_effort_for_model("gemini-pro-orchestrator", "high"),
        None
    );
    assert_eq!(
        model_spec::clamp_effort_for_model("gpt-oss-120b-medium", "high"),
        None
    );

    assert_eq!(
        model_spec::format_pty_model_target("gemini-3.8-flash", None, Some("medium")).as_deref(),
        Some("gemini-3.8-flash-medium")
    );
    assert_eq!(
        model_spec::format_pty_model_target("gemini-3.8-flash", None, None).as_deref(),
        Some("gemini-3.8-flash-high")
    );
    assert_eq!(
        model_spec::format_pty_model_target("gpt-oss-120b-medium", None, Some("high")).as_deref(),
        Some("gpt-oss-120b-medium")
    );
    assert_eq!(
        model_spec::format_pty_model_target("gemini-pro-orchestrator", None, Some("high")).as_deref(),
        Some("gemini-pro-orchestrator")
    );
    assert_eq!(
        model_spec::format_pty_model_target("antigravity-default", Some("high"), None),
        None
    );
}

