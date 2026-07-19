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
fn test_load_from_file_parses_matrix_config() {
    let dir = tempdir().unwrap();
    let config_path = dir.path().join("config.json");
    let json = r#"{
        "matrix": {
            "homeserver": "https://custom.homeserver",
            "user_id": "@bot:custom.homeserver",
            "allowed_rooms": ["!room:custom.homeserver"]
        }
    }"#;
    std::fs::write(&config_path, json).unwrap();

    let config = CliConfig::load_from_file(&config_path).unwrap();
    assert_eq!(config.matrix.homeserver, "https://custom.homeserver");
    assert_eq!(config.matrix.user_id, "@bot:custom.homeserver");
    assert_eq!(config.matrix.allowed_rooms, vec!["!room:custom.homeserver"]);
    assert_eq!(config.matrix.store_path, ".matrix"); // defaulted
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
