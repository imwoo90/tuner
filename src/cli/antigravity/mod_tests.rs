use super::*;
use crate::config::CliConfig;

#[test]
fn test_antigravity_command_uses_print_and_conversation() {
    let config = CliConfig {
        provider: "antigravity".to_string(),
        model: Some("antigravity-default".to_string()),
        ..Default::default()
    };
    let cli = AntigravityCli::new(config);
    let cmd = cli.build_command("hi there", Some("conv-1"), false);

    assert_eq!(cmd[0], "agy");
    assert_eq!(cmd.iter().filter(|&&ref s| s == "--model").count(), 0);
    assert!(cmd.contains(&"--conversation".to_string()));
    assert!(cmd.contains(&"conv-1".to_string()));
    assert_eq!(cmd[cmd.len() - 2..], ["--print".to_string(), "hi there".to_string()]);
}

#[test]
fn test_antigravity_command_grounds_in_workspace() {
    let config = CliConfig {
        provider: "antigravity".to_string(),
        working_dir: PathBuf::from("."),
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
    let config = CliConfig {
        provider: "antigravity".to_string(),
        model: Some("claude-sonnet-4-5".to_string()),
        ..Default::default()
    };
    let cli = AntigravityCli::new(config);
    let cmd = cli.build_command("hi", None, false);

    assert!(cmd.contains(&"--model".to_string()));
    let model_idx = cmd.iter().position(|s| s == "--model").unwrap();
    assert_eq!(cmd[model_idx + 1], "claude-sonnet-4-5");
    
    let print_idx = cmd.iter().position(|s| s == "--print").unwrap();
    assert!(model_idx < print_idx);
}

#[test]
fn test_antigravity_command_continue_and_bypass() {
    let config = CliConfig {
        provider: "antigravity".to_string(),
        permission_mode: "bypassPermissions".to_string(),
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
    let mut cli_params = std::collections::HashMap::new();
    cli_params.insert("antigravity".to_string(), vec!["--log-file".to_string(), "agy.log".to_string()]);
    let config = CliConfig {
        provider: "antigravity".to_string(),
        cli_parameters: cli_params,
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

fn create_test_dir(name: &str) -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("target");
    p.push("test_dirs");
    p.push(name);
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

#[test]
fn test_antigravity_dotted_workspace_mapped_to_symlink() {
    let base = create_test_dir("dotted_test");

    let config = CliConfig {
        provider: "antigravity".to_string(),
        working_dir: base.clone(),
        ..Default::default()
    };
    let cli = AntigravityCli::new(config);
    let agy_ws = cli.agy_workspace();

    let agy_ws_str = agy_ws.to_string_lossy();
    assert!(!agy_ws_str.contains("/."));
    assert_eq!(agy_ws.canonicalize().unwrap(), base.canonicalize().unwrap());

    let cmd = cli.build_command("hi", None, false);
    let idx = cmd.iter().position(|s| s == "--add-dir").unwrap();
    assert_eq!(cmd[idx + 1], agy_ws.to_string_lossy().to_string());
}

#[test]
fn test_antigravity_plain_workspace_unchanged() {
    let plain = PathBuf::from("/home/wimvm/projects/plain_test");

    let config = CliConfig {
        provider: "antigravity".to_string(),
        working_dir: plain.clone(),
        ..Default::default()
    };
    let cli = AntigravityCli::new(config);
    
    assert_eq!(cli.agy_workspace(), plain);
}

#[test]
fn test_format_prompt_injects_workspace_rules_and_memory() {
    let base = create_test_dir("prompt_inject_test");
    
    // Create GEMINI.md
    let gemini_path = base.join("GEMINI.md");
    std::fs::write(&gemini_path, "GEMINI_RULES_CONTENT").unwrap();
    
    // Create memory_system/MAINMEMORY.md
    let mem_dir = base.join("memory_system");
    std::fs::create_dir_all(&mem_dir).unwrap();
    let mem_path = mem_dir.join("MAINMEMORY.md");
    std::fs::write(&mem_path, "MAINMEMORY_CONTENT").unwrap();
    
    let config = CliConfig {
        provider: "antigravity".to_string(),
        working_dir: base.clone(),
        ..Default::default()
    };
    let cli = AntigravityCli::new(config);
    let final_prompt = cli.format_prompt("user_prompt");
    
    assert!(final_prompt.contains("GEMINI_RULES_CONTENT"));
    assert!(final_prompt.contains("MAINMEMORY_CONTENT"));
    assert!(final_prompt.contains("user_prompt"));
}
