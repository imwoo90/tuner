//! # Webhook Manager
//!
//! Manages loading, saving, and updating registered webhooks dynamically with JSON persistence.

//! 
//! ## Search Tags
//! #manager

use crate::webhook::models::WebhookEntry;
use serde_json::Value;
use std::path::PathBuf;
use tokio::sync::RwLock;

pub struct WebhookManager {
    hooks_path: PathBuf,
    hooks: RwLock<Vec<WebhookEntry>>,
    save_lock: tokio::sync::Mutex<()>,
}

impl WebhookManager {
    pub fn new(hooks_path: PathBuf) -> Self {
        let initial_hooks = if hooks_path.exists() {
            if let Ok(data) = std::fs::read_to_string(&hooks_path) {
                if let Ok(wrapper) = serde_json::from_str::<Value>(&data) {
                    wrapper
                        .get("hooks")
                        .and_then(|h| serde_json::from_value::<Vec<WebhookEntry>>(h.clone()).ok())
                        .unwrap_or_default()
                } else {
                    Vec::new()
                }
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };

        Self {
            hooks_path,
            hooks: RwLock::new(initial_hooks),
            save_lock: tokio::sync::Mutex::new(()),
        }
    }

    pub async fn load(&self) -> Result<(), String> {
        if !self.hooks_path.exists() {
            *self.hooks.write().await = Vec::new();
            return Ok(());
        }
        let data = tokio::fs::read_to_string(&self.hooks_path)
            .await
            .map_err(|e| e.to_string())?;
        let wrapper: Value = serde_json::from_str(&data).map_err(|e| e.to_string())?;
        let hook_list = wrapper
            .get("hooks")
            .and_then(|h| serde_json::from_value::<Vec<WebhookEntry>>(h.clone()).ok())
            .unwrap_or_default();
        *self.hooks.write().await = hook_list;
        Ok(())
    }

    pub async fn save(&self) -> Result<(), String> {
        let _guard = self.save_lock.lock().await;
        if let Some(parent) = self.hooks_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| e.to_string())?;
        }
        let hooks = self.hooks.read().await;
        let wrapper = serde_json::json!({ "hooks": *hooks });

        let rand_val: u64 = rand::random();
        let temp_name = format!(
            "{}.{}.tmp",
            self.hooks_path
                .file_name()
                .and_then(|f| f.to_str())
                .unwrap_or("webhooks"),
            rand_val
        );
        let temp_path = self.hooks_path.with_file_name(temp_name);

        let bytes = serde_json::to_vec_pretty(&wrapper).map_err(|e| e.to_string())?;
        if let Err(e) = tokio::fs::write(&temp_path, bytes).await {
            let _ = tokio::fs::remove_file(&temp_path).await;
            return Err(e.to_string());
        }
        if let Err(e) = tokio::fs::rename(&temp_path, &self.hooks_path).await {
            let _ = tokio::fs::remove_file(&temp_path).await;
            return Err(e.to_string());
        }
        Ok(())
    }

    pub async fn add_hook(&self, hook: WebhookEntry) -> Result<(), String> {
        let mut hooks = self.hooks.write().await;
        if hooks.iter().any(|h| h.id == hook.id) {
            return Err(format!("Hook '{}' already exists", hook.id));
        }
        hooks.push(hook);
        drop(hooks);
        self.save().await?;
        Ok(())
    }

    pub async fn remove_hook(&self, hook_id: &str) -> Result<bool, String> {
        let mut hooks = self.hooks.write().await;
        let before = hooks.len();
        hooks.retain(|h| h.id != hook_id);
        let found = hooks.len() != before;
        drop(hooks);
        if found {
            self.save().await?;
        }
        Ok(found)
    }

    pub async fn list_hooks(&self) -> Vec<WebhookEntry> {
        self.hooks.read().await.clone()
    }

    pub async fn get_hook(&self, hook_id: &str) -> Option<WebhookEntry> {
        self.hooks
            .read()
            .await
            .iter()
            .find(|h| h.id == hook_id)
            .cloned()
    }

    pub async fn update_hook(&self, hook_id: &str, updates: &Value) -> Result<bool, String> {
        let mut hooks = self.hooks.write().await;
        let Some(hook) = hooks.iter_mut().find(|h| h.id == hook_id) else {
            return Ok(false);
        };

        if let Some(map) = updates.as_object() {
            for (key, val) in map {
                update_basic_fields(hook, key, val);
                update_auth_fields(hook, key, val);
                update_execution_fields(hook, key, val);
            }
        }

        drop(hooks);
        self.save().await?;
        Ok(true)
    }
}

fn update_basic_fields(hook: &mut WebhookEntry, key: &str, val: &Value) {
    match key {
        "title" => {
            if let Some(s) = val.as_str() {
                hook.title = s.to_string();
            }
        }
        "description" => {
            if let Some(s) = val.as_str() {
                hook.description = s.to_string();
            }
        }
        "mode" => {
            if let Some(s) = val.as_str() {
                hook.mode = s.to_string();
            }
        }
        "prompt_template" => {
            if let Some(s) = val.as_str() {
                hook.prompt_template = s.to_string();
            }
        }
        "enabled" => {
            if let Some(b) = val.as_bool() {
                hook.enabled = b;
            }
        }
        "task_folder" => {
            hook.task_folder = val.as_str().map(|s| s.to_string());
        }
        _ => {}
    }
}

fn update_auth_fields(hook: &mut WebhookEntry, key: &str, val: &Value) {
    match key {
        "auth_mode" => {
            if let Some(s) = val.as_str() {
                hook.auth_mode = s.to_string();
            }
        }
        "token" => {
            if let Some(s) = val.as_str() {
                hook.token = s.to_string();
            }
        }
        "hmac_secret" => {
            if let Some(s) = val.as_str() {
                hook.hmac_secret = s.to_string();
            }
        }
        "hmac_header" => {
            if let Some(s) = val.as_str() {
                hook.hmac_header = s.to_string();
            }
        }
        "hmac_algorithm" => {
            if let Some(s) = val.as_str() {
                hook.hmac_algorithm = s.to_string();
            }
        }
        "hmac_encoding" => {
            if let Some(s) = val.as_str() {
                hook.hmac_encoding = s.to_string();
            }
        }
        "hmac_sig_prefix" => {
            if let Some(s) = val.as_str() {
                hook.hmac_sig_prefix = s.to_string();
            }
        }
        "hmac_sig_regex" => {
            if let Some(s) = val.as_str() {
                hook.hmac_sig_regex = s.to_string();
            }
        }
        "hmac_payload_prefix_regex" => {
            if let Some(s) = val.as_str() {
                hook.hmac_payload_prefix_regex = s.to_string();
            }
        }
        _ => {}
    }
}

fn update_execution_fields(hook: &mut WebhookEntry, key: &str, val: &Value) {
    match key {
        "provider" => {
            hook.provider = val.as_str().map(|s| s.to_string());
        }
        "model" => {
            hook.model = val.as_str().map(|s| s.to_string());
        }
        "reasoning_effort" => {
            hook.reasoning_effort = val.as_str().map(|s| s.to_string());
        }
        "cli_parameters" => {
            if let Some(arr) = val.as_array() {
                hook.cli_parameters = arr
                    .iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect();
            }
        }
        "quiet_start" => {
            hook.quiet_start = val.as_u64().map(|n| n as u32);
        }
        "quiet_end" => {
            hook.quiet_end = val.as_u64().map(|n| n as u32);
        }
        "dependency" => {
            hook.dependency = val.as_str().map(|s| s.to_string());
        }
        _ => {}
    }
}

impl WebhookManager {
    pub async fn record_trigger(&self, hook_id: &str, error: Option<String>) {
        let mut hooks = self.hooks.write().await;
        if let Some(hook) = hooks.iter_mut().find(|h| h.id == hook_id) {
            hook.trigger_count += 1;
            hook.last_triggered_at = Some(chrono::Utc::now().to_rfc3339());
            hook.last_error = error;
            drop(hooks);
            let _ = self.save().await;
        }
    }

    pub async fn reload(&self) -> Result<(), String> {
        self.load().await
    }
}
