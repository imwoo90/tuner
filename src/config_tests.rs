use crate::config::*;
use std::path::PathBuf;
use tempfile::tempdir;

#[test]
fn test_load_from_file_parses_json_with_defaults() {
    let dir = tempdir().unwrap();
    let config_path = dir.path().join("config.json");
    let json = r#"{
        "provider": "antigravity",
        "telegram_token": "123:abc",
        "allowed_user_ids": [456],
        "allowed_group_ids": [-789]
    }"#;
    std::fs::write(&config_path, json).unwrap();

    let config = CliConfig::load_from_file(&config_path).unwrap();
    assert_eq!(config.provider, "antigravity");
    assert_eq!(config.telegram_token, "123:abc");
    assert_eq!(config.allowed_user_ids, vec![456]);
    assert_eq!(config.allowed_group_ids, vec![-789]);
    assert_eq!(config.working_dir, PathBuf::from(".")); // defaulted
}

#[test]
fn test_load_from_file_parses_profiles_array() {
    let dir = tempdir().unwrap();
    let config_path = dir.path().join("config.json");
    let json = r#"{
        "profiles": [
            {
                "name": "inmyung",
                "telegram_token": "token1",
                "allowed_user_ids": [111]
            },
            {
                "name": "seojin",
                "telegram_token": "token2",
                "allowed_user_ids": [222]
            }
        ]
    }"#;
    std::fs::write(&config_path, json).unwrap();

    let config = CliConfig::load_from_file(&config_path).unwrap();
    assert_eq!(config.profiles.len(), 2);
    assert_eq!(config.profiles[0].name, "inmyung");
    assert_eq!(config.profiles[0].telegram_token, "token1");
    assert_eq!(config.profiles[0].allowed_user_ids, vec![111]);
    assert_eq!(config.profiles[1].name, "seojin");
    assert_eq!(config.profiles[1].telegram_token, "token2");
    assert_eq!(config.profiles[1].allowed_user_ids, vec![222]);
}

#[test]
fn test_merge_profile_file_overrides_fields() {
    let dir = tempdir().unwrap();
    let config_path = dir.path().join("config.json");
    let global_json = r#"{
        "provider": "antigravity",
        "telegram_token": "global_token",
        "allowed_user_ids": [111],
        "allowed_group_ids": [-111]
    }"#;
    std::fs::write(&config_path, global_json).unwrap();

    let mut config = CliConfig::load_from_file(&config_path).unwrap();
    assert_eq!(config.telegram_token, "global_token");

    let profile_json_path = dir.path().join("profile.json");
    let profile_json = r#"{
        "telegram_token": "profile_token",
        "allowed_user_ids": [222],
        "language": "ko"
    }"#;
    std::fs::write(&profile_json_path, profile_json).unwrap();

    config.merge_profile_file(&profile_json_path).unwrap();
    assert_eq!(config.telegram_token, "profile_token");
    assert_eq!(config.allowed_user_ids, vec![222]); // overridden
    assert_eq!(config.allowed_group_ids, vec![-111]); // preserved from global
    assert_eq!(config.language, Some("ko".to_string())); // overridden
}

#[test]
fn test_find_agy_vault_bin_resolution() {
    let dir = tempfile::tempdir().unwrap();
    let home_str = dir.path().to_string_lossy().to_string();
    let tuner_bin_dir = dir.path().join(".tuner/bin");
    std::fs::create_dir_all(&tuner_bin_dir).unwrap();
    let fake_vault = tuner_bin_dir.join("agy-vault");
    std::fs::write(&fake_vault, "#!/bin/sh\n").unwrap();

    let found = crate::setup::find_agy_vault_bin(&home_str);
    assert_eq!(found, Some(fake_vault));
}

#[test]
fn test_override_profile_config_copies_effort_and_model() {
    let dir = tempfile::tempdir().unwrap();
    let paths = crate::workspace::paths::TunerPaths::new(
        dir.path().to_path_buf(),
        dir.path().join("defaults"),
        Some("default".to_string()),
    );
    let mut cfg = CliConfig::default();
    cfg.profiles.push(ProfileConfig {
        name: "default".to_string(),
        telegram_token: "123:tok".to_string(),
        allowed_user_ids: vec![42],
        allowed_group_ids: vec![-100],
        working_dir: None,
        model: Some("gemini-4-argon".to_string()),
        effort: Some("high".to_string()),
        system_prompt: None,
        append_system_prompt: None,
        language: Some("ko".to_string()),
    });
    crate::setup::override_profile_config(&mut cfg, "default", &paths).unwrap();
    assert_eq!(cfg.model.as_deref(), Some("gemini-4-argon"));
    assert_eq!(cfg.effort.as_deref(), Some("high"));
    assert_eq!(cfg.language.as_deref(), Some("ko"));
}

#[test]
fn test_load_from_file_strips_template_placeholder_ids() {
    let dir = tempdir().unwrap();
    let config_path = dir.path().join("config.json");
    let json = r#"{
        "telegram_token": "YOUR_BOT_TOKEN_HERE",
        "allowed_user_ids": [123456789],
        "allowed_group_ids": [-1001234567890]
    }"#;
    std::fs::write(&config_path, json).unwrap();

    let config = CliConfig::load_from_file(&config_path).unwrap();
    assert!(config.allowed_user_ids.is_empty());
    assert!(config.allowed_group_ids.is_empty());
    assert!(config.profiles[0].allowed_user_ids.is_empty());
    assert!(config.profiles[0].allowed_group_ids.is_empty());
}
