//! # Webhook Data Models and Rendering
//!
//! Defines the data structures and templates for webhook registration.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WebhookEntry {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    pub mode: String, // "wake" | "cron_task"
    pub prompt_template: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub task_folder: Option<String>,
    #[serde(default = "default_bearer")]
    pub auth_mode: String,
    #[serde(default)]
    pub token: String,
    #[serde(default)]
    pub hmac_secret: String,
    #[serde(default)]
    pub hmac_header: String,
    #[serde(default = "default_sha256")]
    pub hmac_algorithm: String,
    #[serde(default = "default_hex")]
    pub hmac_encoding: String,
    #[serde(default = "default_sig_prefix")]
    pub hmac_sig_prefix: String,
    #[serde(default)]
    pub hmac_sig_regex: String,
    #[serde(default)]
    pub hmac_payload_prefix_regex: String,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub trigger_count: u64,
    pub last_triggered_at: Option<String>,
    pub last_error: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub reasoning_effort: Option<String>,
    #[serde(default)]
    pub cli_parameters: Vec<String>,
    pub quiet_start: Option<u32>,
    pub quiet_end: Option<u32>,
    pub dependency: Option<String>,
}

fn default_true() -> bool {
    true
}

fn default_bearer() -> String {
    "bearer".to_string()
}

fn default_sha256() -> String {
    "sha256".to_string()
}

fn default_hex() -> String {
    "hex".to_string()
}

fn default_sig_prefix() -> String {
    "sha256=".to_string()
}

#[derive(Debug, Clone)]
pub struct WebhookResult {
    pub hook_id: String,
    pub hook_title: String,
    pub mode: String,
    pub result_text: String,
    pub status: String, // "success" | "error:..."
}

/// Replace `{{field}}` placeholders with values from `payload`.
///
/// Missing keys render as `{{?field}}` so they are visible but non-fatal.
pub fn render_template(template: &str, payload: &serde_json::Value) -> String {
    let re = regex::Regex::new(r"\{\{(\w+)\}\}").unwrap();
    re.replace_all(template, |caps: &regex::Captures| {
        let key = &caps[1];
        match payload.get(key) {
            None | Some(serde_json::Value::Null) => format!("{{{{?{}}}}}", key),
            Some(serde_json::Value::String(s)) => s.clone(),
            Some(serde_json::Value::Number(n)) => n.to_string(),
            Some(serde_json::Value::Bool(b)) => b.to_string(),
            Some(other) => other.to_string(),
        }
    })
    .into_owned()
}
