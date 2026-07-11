use std::path::{Path, PathBuf};
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct CliConfig {
    pub provider: String,
    pub working_dir: PathBuf,
    pub model: Option<String>,
    pub system_prompt: Option<String>,
    pub append_system_prompt: Option<String>,
    pub permission_mode: String,
    pub docker_container: Option<String>,
    pub cli_parameters: std::collections::HashMap<String, Vec<String>>,
    pub chat_id: i64,
    pub topic_id: Option<i64>,
    pub process_label: String,
    pub transport: String,
    pub telegram_token: String,
    pub allowed_user_ids: Vec<i64>,
    pub allowed_group_ids: Vec<i64>,
    pub user_timezone: Option<String>,
    pub telegram_heartbeat_enabled: bool,
    pub telegram_heartbeat_interval_minutes: Option<i64>,
    pub telegram_heartbeat_quiet_start: Option<u32>,
    pub telegram_heartbeat_quiet_end: Option<u32>,
    pub telegram_heartbeat_ack_token: Option<String>,
    pub heartbeat: HeartbeatConfig,
    pub cleanup: crate::cleanup::observer::CleanupConfig,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct HeartbeatConfig {
    pub enabled: bool,
    pub interval_minutes: Option<i64>,
    pub quiet_start: Option<u32>,
    pub quiet_end: Option<u32>,
    pub ack_token: Option<String>,
}

impl Default for HeartbeatConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_minutes: Some(30),
            quiet_start: Some(21),
            quiet_end: Some(8),
            ack_token: Some("HEARTBEAT_OK".to_string()),
        }
    }
}

impl Default for CliConfig {
    fn default() -> Self {
        Self {
            provider: "claude".to_string(),
            working_dir: PathBuf::from("."),
            model: None,
            system_prompt: None,
            append_system_prompt: None,
            permission_mode: "bypassPermissions".to_string(),
            docker_container: None,
            cli_parameters: std::collections::HashMap::new(),
            chat_id: 0,
            topic_id: None,
            process_label: "main".to_string(),
            transport: "tg".to_string(),
            telegram_token: String::new(),
            allowed_user_ids: Vec::new(),
            allowed_group_ids: Vec::new(),
            user_timezone: None,
            telegram_heartbeat_enabled: false,
            telegram_heartbeat_interval_minutes: None,
            telegram_heartbeat_quiet_start: None,
            telegram_heartbeat_quiet_end: None,
            telegram_heartbeat_ack_token: None,
            heartbeat: HeartbeatConfig::default(),
            cleanup: crate::cleanup::observer::CleanupConfig::default(),
        }
    }
}

impl CliConfig {
    pub fn load_from_file(path: &Path) -> Result<Self, String> {
        let content = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        serde_json::from_str(&content).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
}
