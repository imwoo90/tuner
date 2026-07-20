//! # Configuration Layer for Tuner Daemon
//!
//! Handles JSON-based configurations, profile overrides, loading environment variables
//! from local settings, systemd integration checks, and validation of user credentials.

use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
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
    pub webhooks: WebhookConfig,
    pub api: ApiConfig,
    pub matrix: MatrixConfig,
    pub language: Option<String>,
    pub profiles: Vec<ProfileConfig>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct MatrixConfig {
    pub homeserver: String,
    pub user_id: String,
    pub password: Option<String>,
    pub access_token: Option<String>,
    pub device_id: Option<String>,
    pub allowed_rooms: Vec<String>,
    pub allowed_users: Vec<String>,
    pub store_path: String,
}

impl Default for MatrixConfig {
    fn default() -> Self {
        Self {
            homeserver: "https://matrix.org".to_string(),
            user_id: String::new(),
            password: None,
            access_token: None,
            device_id: None,
            allowed_rooms: Vec::new(),
            allowed_users: Vec::new(),
            store_path: ".matrix".to_string(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct WebhookConfig {
    pub enabled: bool,
    pub host: String,
    pub port: u16,
    pub token: String,
    pub max_body_bytes: usize,
    pub rate_limit_per_minute: usize,
}

impl Default for WebhookConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            host: "127.0.0.1".to_string(),
            port: 8742,
            token: String::new(),
            max_body_bytes: 262144,
            rate_limit_per_minute: 30,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct ApiConfig {
    pub enabled: bool,
    pub host: String,
    pub port: u16,
    pub token: String,
    pub chat_id: i64,
    pub allow_public: bool,
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            host: "0.0.0.0".to_string(),
            port: 8741,
            token: String::new(),
            chat_id: 0,
            allow_public: false,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
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
            webhooks: WebhookConfig::default(),
            api: ApiConfig::default(),
            matrix: MatrixConfig::default(),
            language: None,
            profiles: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct ProfileConfig {
    pub name: String,
    pub telegram_token: String,
    pub allowed_user_ids: Vec<i64>,
    pub allowed_group_ids: Vec<i64>,
    pub working_dir: Option<PathBuf>,
    pub model: Option<String>,
    pub system_prompt: Option<String>,
    pub append_system_prompt: Option<String>,
    pub language: Option<String>,
}

impl Default for ProfileConfig {
    fn default() -> Self {
        Self {
            name: "default".to_string(),
            telegram_token: String::new(),
            allowed_user_ids: Vec::new(),
            allowed_group_ids: Vec::new(),
            working_dir: None,
            model: None,
            system_prompt: None,
            append_system_prompt: None,
            language: None,
        }
    }
}

impl CliConfig {
    pub fn load_from_file(path: &Path) -> Result<Self, String> {
        let content = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        let mut cfg: Self = serde_json::from_str(&content).map_err(|e| e.to_string())?;
        if cfg.profiles.is_empty() {
            let default_profile = ProfileConfig {
                name: "default".to_string(),
                telegram_token: cfg.telegram_token.clone(),
                allowed_user_ids: cfg.allowed_user_ids.clone(),
                allowed_group_ids: cfg.allowed_group_ids.clone(),
                working_dir: Some(cfg.working_dir.clone()),
                model: cfg.model.clone(),
                system_prompt: cfg.system_prompt.clone(),
                append_system_prompt: cfg.append_system_prompt.clone(),
                language: cfg.language.clone(),
            };
            cfg.profiles.push(default_profile);
        }
        Ok(cfg)
    }

    pub fn merge_profile_file(&mut self, path: &Path) -> Result<(), String> {
        let content = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        let mut overrides: serde_json::Value = serde_json::from_str(&content).map_err(|e| e.to_string())?;
        
        if let Some(obj) = overrides.as_object_mut() {
            if let Some(tok) = obj.get("telegram_token").and_then(|t| t.as_str()) {
                if tok.is_empty() || tok == "YOUR_BOT_TOKEN_HERE" || tok.starts_with("YOUR_") {
                    obj.remove("telegram_token");
                }
            }
            if let Some(ids) = obj.get("allowed_user_ids").and_then(|i| i.as_array()) {
                if ids.len() == 1 && ids[0] == serde_json::Value::Number(123456789.into()) {
                    obj.remove("allowed_user_ids");
                }
            }
            if let Some(ids) = obj.get("allowed_group_ids").and_then(|i| i.as_array()) {
                if ids.len() == 1 && ids[0] == serde_json::Value::Number((-1001234567890i64).into()) {
                    obj.remove("allowed_group_ids");
                }
            }
        }

        let mut base_val = serde_json::to_value(&self).map_err(|e| e.to_string())?;
        merge_json_values(&mut base_val, overrides);
        
        *self = serde_json::from_value(base_val).map_err(|e| e.to_string())?;
        Ok(())
    }
}

fn merge_json_values(base: &mut serde_json::Value, overrides: serde_json::Value) {
    if let (Some(base_obj), Some(over_obj)) = (base.as_object_mut(), overrides.as_object()) {
        for (k, v) in over_obj {
            if v.is_object() && base_obj.contains_key(k) && base_obj[k].is_object() {
                merge_json_values(&mut base_obj[k], v.clone());
            } else if !v.is_null() {
                base_obj.insert(k.clone(), v.clone());
            }
        }
    }
}


