//! # Config Loader and User Validator
//!
//! ## Overview
//! Manages loading and merging JSON configurations. Registers telegram credentials
//! and allowed roots boundary parameters.
//!
//! ## Search Tags
//! #config-loader, #json-config, #profile-merger

use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
/// Represents `CliConfig` (cli config).
pub struct CliConfig {
    /// Provider.
    pub provider: String,
    /// Working dir.
    pub working_dir: PathBuf,
    /// Model.
    pub model: Option<String>,
    /// Effort.
    pub effort: Option<String>,
    /// System prompt.
    pub system_prompt: Option<String>,
    /// Append system prompt.
    pub append_system_prompt: Option<String>,
    /// Permission mode.
    pub permission_mode: String,
    /// Cli parameters.
    pub cli_parameters: std::collections::HashMap<String, Vec<String>>,
    /// Chat id.
    pub chat_id: i64,
    /// Topic id.
    pub topic_id: Option<i64>,
    /// Transport.
    pub transport: String,
    /// Telegram token.
    pub telegram_token: String,
    /// Allowed user ids.
    pub allowed_user_ids: Vec<i64>,
    /// Allowed group ids.
    pub allowed_group_ids: Vec<i64>,
    /// User timezone.
    pub user_timezone: Option<String>,
    /// Idle timeout minutes.
    pub idle_timeout_minutes: i64,
    /// Daily reset hour.
    pub daily_reset_hour: u32,
    /// Daily reset enabled.
    pub daily_reset_enabled: bool,
    /// Max session messages.
    pub max_session_messages: Option<i64>,
    /// Heartbeat.
    pub heartbeat: HeartbeatConfig,
    /// Cleanup.
    pub cleanup: crate::cleanup::observer::CleanupConfig,
    /// Language.
    pub language: Option<String>,
    /// Profiles.
    pub profiles: Vec<ProfileConfig>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
/// Represents `HeartbeatConfig` (heartbeat config).
pub struct HeartbeatConfig {
    /// Enabled.
    pub enabled: bool,
    /// Interval minutes.
    pub interval_minutes: Option<i64>,
    /// Quiet start.
    pub quiet_start: Option<u32>,
    /// Quiet end.
    pub quiet_end: Option<u32>,
    /// Ack token.
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
            provider: "antigravity".to_string(),
            working_dir: PathBuf::from("."),
            model: None,
            effort: None,
            system_prompt: None,
            append_system_prompt: None,
            permission_mode: "bypassPermissions".to_string(),
            cli_parameters: std::collections::HashMap::new(),
            chat_id: 0,
            topic_id: None,
            transport: "tg".to_string(),
            telegram_token: String::new(),
            allowed_user_ids: Vec::new(),
            allowed_group_ids: Vec::new(),
            user_timezone: None,
            idle_timeout_minutes: 0,
            daily_reset_hour: 4,
            daily_reset_enabled: false,
            max_session_messages: None,
            heartbeat: HeartbeatConfig::default(),
            cleanup: crate::cleanup::observer::CleanupConfig::default(),
            language: None,
            profiles: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
/// Represents `ProfileConfig` (profile config).
pub struct ProfileConfig {
    /// Name.
    pub name: String,
    /// Telegram token.
    pub telegram_token: String,
    /// Allowed user ids.
    pub allowed_user_ids: Vec<i64>,
    /// Allowed group ids.
    pub allowed_group_ids: Vec<i64>,
    /// Working dir.
    pub working_dir: Option<PathBuf>,
    /// Model.
    pub model: Option<String>,
    /// Effort.
    pub effort: Option<String>,
    /// System prompt.
    pub system_prompt: Option<String>,
    /// Append system prompt.
    pub append_system_prompt: Option<String>,
    /// Language.
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
            effort: None,
            system_prompt: None,
            append_system_prompt: None,
            language: None,
        }
    }
}

impl CliConfig {
    /// Executes `load_from_file` (load from file).
    pub fn load_from_file(path: &Path) -> Result<Self, String> {
        let content = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        let mut cfg: Self = serde_json::from_str(&content).map_err(|e| e.to_string())?;
        if cfg.allowed_user_ids == vec![123456789] {
            cfg.allowed_user_ids.clear();
        }
        if cfg.allowed_group_ids == vec![-1001234567890] {
            cfg.allowed_group_ids.clear();
        }
        if cfg.profiles.is_empty() {
            let default_profile = ProfileConfig {
                name: "default".to_string(),
                telegram_token: cfg.telegram_token.clone(),
                allowed_user_ids: cfg.allowed_user_ids.clone(),
                allowed_group_ids: cfg.allowed_group_ids.clone(),
                working_dir: Some(cfg.working_dir.clone()),
                model: cfg.model.clone(),
                effort: cfg.effort.clone(),
                system_prompt: cfg.system_prompt.clone(),
                append_system_prompt: cfg.append_system_prompt.clone(),
                language: cfg.language.clone(),
            };
            cfg.profiles.push(default_profile);
        }
        Ok(cfg)
    }

    /// Executes `merge_profile_file` (merge profile file).
    pub fn merge_profile_file(&mut self, path: &Path) -> Result<(), String> {
        let content = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        let mut overrides: serde_json::Value = serde_json::from_str(&content).map_err(|e| e.to_string())?;
        
        if let Some(obj) = overrides.as_object_mut() {
            if let Some(tok) = obj.get("telegram_token").and_then(|t| t.as_str())
                && (tok.is_empty() || tok == "YOUR_BOT_TOKEN_HERE" || tok.starts_with("YOUR_")) {
                    obj.remove("telegram_token");
                }
            if let Some(ids) = obj.get("allowed_user_ids").and_then(|i| i.as_array())
                && ids.len() == 1 && ids[0] == serde_json::Value::Number(123456789.into()) {
                    obj.remove("allowed_user_ids");
                }
            if let Some(ids) = obj.get("allowed_group_ids").and_then(|i| i.as_array())
                && ids.len() == 1 && ids[0] == serde_json::Value::Number((-1001234567890i64).into()) {
                    obj.remove("allowed_group_ids");
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


