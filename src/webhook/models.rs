//! # Webhook Data Models and Rendering
//!
//! ## Overview
//! Defines the data structures and templates for webhook registration.
//! Defines serializable structures for hook definitions, execution modes, and incoming HTTP request payloads.
//!
//! ## Search Tags
//! #models

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
/// Represents `WebhookEntry` (webhook entry).
pub struct WebhookEntry {
    /// Id.
    pub id: String,
    /// Title.
    pub title: String,
    #[serde(default)]
    /// Description.
    pub description: String,
    /// Mode.
    pub mode: String, // "wake" | "cron_task"
    /// Prompt template.
    pub prompt_template: String,
    #[serde(default = "default_true")]
    /// Enabled.
    pub enabled: bool,
    /// Task folder.
    pub task_folder: Option<String>,
    #[serde(default = "default_bearer")]
    /// Auth mode.
    pub auth_mode: String,
    #[serde(default)]
    /// Token.
    pub token: String,
    #[serde(default)]
    /// Hmac secret.
    pub hmac_secret: String,
    #[serde(default)]
    /// Hmac header.
    pub hmac_header: String,
    #[serde(default = "default_sha256")]
    /// Hmac algorithm.
    pub hmac_algorithm: String,
    #[serde(default = "default_hex")]
    /// Hmac encoding.
    pub hmac_encoding: String,
    #[serde(default = "default_sig_prefix")]
    /// Hmac sig prefix.
    pub hmac_sig_prefix: String,
    #[serde(default)]
    /// Hmac sig regex.
    pub hmac_sig_regex: String,
    #[serde(default)]
    /// Hmac payload prefix regex.
    pub hmac_payload_prefix_regex: String,
    #[serde(skip)]
    #[serde(default)]
    /// Hmac sig regex cached.
    pub hmac_sig_regex_cached: std::sync::Arc<std::sync::OnceLock<Option<regex::Regex>>>,
    #[serde(skip)]
    #[serde(default)]
    /// Hmac payload prefix regex cached.
    pub hmac_payload_prefix_regex_cached: std::sync::Arc<std::sync::OnceLock<Option<regex::Regex>>>,
    #[serde(default)]
    /// Created at.
    pub created_at: String,
    #[serde(default)]
    /// Trigger count.
    pub trigger_count: u64,
    /// Last triggered at.
    pub last_triggered_at: Option<String>,
    /// Last error.
    pub last_error: Option<String>,
    /// Provider.
    pub provider: Option<String>,
    /// Model.
    pub model: Option<String>,
    /// Reasoning effort.
    pub reasoning_effort: Option<String>,
    #[serde(default)]
    /// Cli parameters.
    pub cli_parameters: Vec<String>,
    /// Quiet start.
    pub quiet_start: Option<u32>,
    /// Quiet end.
    pub quiet_end: Option<u32>,
    /// Dependency.
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
/// Represents `WebhookResult` (webhook result).
pub struct WebhookResult {
    /// Hook id.
    pub hook_id: String,
    /// Hook title.
    pub hook_title: String,
    /// Mode.
    pub mode: String,
    /// Result text.
    pub result_text: String,
    /// Status.
    pub status: String, // "success" | "error:..."
}

/// Replace `{{field}}` placeholders with values from `payload`.
///
/// Missing keys render as `{{?field}}` so they are visible but non-fatal.
pub fn render_template(template: &str, payload: &serde_json::Value) -> String {
    static TEMPLATE_RE: std::sync::OnceLock<Option<regex::Regex>> = std::sync::OnceLock::new();
    let Some(re) = TEMPLATE_RE.get_or_init(|| regex::Regex::new(r"\{\{(\w+)\}\}").ok()) else {
        return template.to_string();
    };
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
