//! Task Data Models
//!
//! Struct definitions for task submissions, persisted entries, results, and priorities.

//! 
//! ## Search Tags
//! #models

use serde::{Deserialize, Serialize};

/// Allowed task priority levels
pub const TASK_PRIORITIES: &[&str] = &["interactive", "background", "batch"];
/// Default task priority level
pub const DEFAULT_PRIORITY: &str = "background";

/// Configuration settings for the task system
#[derive(Clone, Debug)]
pub struct TasksConfig {
    pub enabled: bool,
    pub max_parallel: usize,
    pub timeout_seconds: f64,
}

impl Default for TasksConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_parallel: 5,
            timeout_seconds: 60.0,
        }
    }
}

/// Coerces a string slice to a valid task priority, or defaults to "background"
pub fn normalise_priority(value: Option<&str>) -> String {
    match value {
        Some(p) if TASK_PRIORITIES.contains(&p) => p.to_string(),
        _ => DEFAULT_PRIORITY.to_string(),
    }
}

fn default_parent_agent() -> String {
    "main".to_string()
}

fn default_status() -> String {
    "running".to_string()
}

fn default_priority() -> String {
    DEFAULT_PRIORITY.to_string()
}

/// A persisted task entry in the registry
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TaskEntry {
    pub task_id: String,
    pub chat_id: i64,
    #[serde(default = "default_parent_agent")]
    pub parent_agent: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub prompt_preview: String,
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub model: String,
    #[serde(default = "default_status")]
    pub status: String, // "running", "done", "failed", "cancelled", "waiting"
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub created_at: f64,
    #[serde(default)]
    pub completed_at: f64,
    #[serde(default)]
    pub elapsed_seconds: f64,
    #[serde(default)]
    pub error: String,
    #[serde(default)]
    pub result_preview: String,
    #[serde(default)]
    pub question_count: i64,
    #[serde(default)]
    pub num_turns: i64,
    #[serde(default)]
    pub last_question: String,
    #[serde(default)]
    pub original_prompt: String,
    #[serde(default)]
    pub thinking: String,
    #[serde(default)]
    pub tasks_dir: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<i64>,
    #[serde(default = "default_priority", deserialize_with = "deserialize_priority")]
    pub priority: String,
    #[serde(default)]
    pub depends_on: Vec<String>,
}

fn deserialize_priority<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let opt: Option<String> = Deserialize::deserialize(deserializer)?;
    Ok(normalise_priority(opt.as_deref()))
}

/// Represents a submitted task
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskSubmit {
    pub chat_id: i64,
    pub prompt: String,
    pub message_id: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<i64>,
    pub parent_agent: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub provider_override: String,
    #[serde(default)]
    pub model_override: String,
    #[serde(default)]
    pub thinking_override: String,
    #[serde(default = "default_priority")]
    pub priority: String,
    #[serde(default)]
    pub depends_on: Vec<String>,
}

/// Outcome delivered after task completes
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskResult {
    pub task_id: String,
    pub chat_id: i64,
    pub parent_agent: String,
    pub name: String,
    pub prompt_preview: String,
    pub result_text: String,
    pub status: String,
    pub elapsed_seconds: f64,
    pub provider: String,
    pub model: String,
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub error: String,
    #[serde(default)]
    pub task_folder: String,
    #[serde(default)]
    pub original_prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<i64>,
}
