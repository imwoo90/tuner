//! # Webhook Manager
//!
//! ## Overview
//! Manages loading, saving, and updating registered webhooks dynamically with JSON persistence.
//! Provides thread-safe CRUD operations and atomic disk persistence for webhook endpoint configurations.
//!
//! ## Search Tags
//! #manager

use crate::webhook::models::WebhookEntry;
use serde_json::Value;
use std::path::PathBuf;
use tokio::sync::RwLock;

/// Represents `WebhookManager` (webhook manager).
pub struct WebhookManager {
    hooks_path: PathBuf,
    hooks: RwLock<Vec<WebhookEntry>>,
    save_lock: tokio::sync::Mutex<()>,
}

impl WebhookManager {
    /// Creates a new instance.
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

    /// Executes `load` (load).
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

    /// Executes `save` (save).
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

    /// Executes `get_hook` (get hook).
    pub async fn get_hook(&self, hook_id: &str) -> Option<WebhookEntry> {
        self.hooks
            .read()
            .await
            .iter()
            .find(|h| h.id == hook_id)
            .cloned()
    }

    /// Executes `record_trigger` (record trigger).
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

    /// Executes `reload` (reload).
    pub async fn reload(&self) -> Result<(), String> {
        self.load().await
    }
}
