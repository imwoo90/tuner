//! # Adapters Module
//!
//! Provides conversion functions to map various domain events and results
//! (e.g., CronResult, BackgroundResult, InterAgentResult, TaskResult) into unified `Envelope` models.

//! 
//! ## Search Tags
//! #adapters

use super::envelope::{Envelope, Origin, LockMode, DeliveryMode};
use crate::background::models::{BackgroundResult, BackgroundResultStatus};

#[derive(Debug, Clone)]
pub struct WebhookResult {
    pub hook_id: String,
    pub hook_title: String,
    pub result_text: String,
    pub status: String,
}

#[derive(Debug, Clone)]
pub struct InterAgentResult {
    pub task_id: String,
    pub sender: String,
    pub recipient: String,
    pub message_preview: String,
    pub result_text: String,
    pub success: bool,
    pub error: Option<String>,
    pub elapsed_seconds: f64,
    pub session_name: String,
    pub provider_switch_notice: String,
    pub original_message: String,
    pub chat_id: i64,
    pub topic_id: Option<i64>,
}

#[derive(Debug, Clone)]
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
    pub session_id: String,
    pub error: String,
    pub task_folder: String,
    pub original_prompt: String,
    pub thread_id: Option<i64>,
}

pub fn from_background_result(result: &BackgroundResult) -> Envelope {
    let status_str = match result.status {
        BackgroundResultStatus::Success => "success".to_string(),
        BackgroundResultStatus::Aborted => "aborted".to_string(),
        BackgroundResultStatus::ErrorCli => "error:cli".to_string(),
        BackgroundResultStatus::ErrorTimeout => "error:timeout".to_string(),
        BackgroundResultStatus::ErrorCliNotFound => "error:cli_not_found".to_string(),
        BackgroundResultStatus::ErrorInternal => "error:internal".to_string(),
    };
    let is_error = status_str.starts_with("error:");
    let mut env = Envelope::new(Origin::Background, result.chat_id);
    env.reply_to_message_id = Some(result.message_id);
    env.thread_id = result.thread_id;
    env.topic_id = result.thread_id;
    env.prompt_preview = result.prompt_preview.clone();
    env.result_text = result.result_text.clone();
    env.status = status_str;
    env.is_error = is_error;
    env.elapsed_seconds = result.elapsed_seconds;
    env
}

pub fn from_cron_result(
    title: &str,
    result_text: &str,
    status: &str,
    chat_id: Option<i64>,
    topic_id: Option<i64>,
    transport: Option<&str>,
) -> Envelope {
    let mut env = Envelope::new(Origin::Cron, chat_id.unwrap_or(0));
    env.topic_id = topic_id;
    env.result_text = result_text.to_string();
    env.status = status.to_string();
    env.is_error = status.to_lowercase().contains("error");
    if chat_id.is_some() {
        env.delivery = DeliveryMode::Unicast;
    } else {
        env.delivery = DeliveryMode::Broadcast;
    }
    if let Some(t) = transport {
        env.transport = t.to_string();
    }
    env.metadata.insert("title".to_string(), title.to_string());
    env
}

pub fn from_heartbeat(
    chat_id: i64,
    alert_text: &str,
    topic_id: Option<i64>,
    transport: Option<&str>,
) -> Envelope {
    let mut env = Envelope::new(Origin::Heartbeat, chat_id);
    env.topic_id = topic_id;
    env.result_text = alert_text.to_string();
    env.status = "success".to_string();
    if let Some(t) = transport {
        env.transport = t.to_string();
    }
    env
}

pub fn from_webhook_cron_result(result: &WebhookResult) -> Envelope {
    let mut env = Envelope::new(Origin::WebhookCron, 0);
    env.result_text = result.result_text.clone();
    env.status = result.status.clone();
    env.is_error = result.status.to_lowercase().contains("error");
    env.delivery = DeliveryMode::Broadcast;
    env.metadata.insert("hook_title".to_string(), result.hook_title.clone());
    env.metadata.insert("hook_id".to_string(), result.hook_id.clone());
    env
}

pub fn from_webhook_wake(chat_id: i64, prompt: &str) -> Envelope {
    let mut env = Envelope::new(Origin::WebhookWake, chat_id);
    env.prompt = prompt.to_string();
    env.delivery = DeliveryMode::Unicast;
    env.lock_mode = LockMode::Required;
    env
}

pub fn from_interagent_result(
    result: &InterAgentResult,
    chat_id: i64,
    injection_prompt: Option<&str>,
    transport: Option<&str>,
) -> Envelope {
    let target_chat_id = if result.chat_id != 0 { result.chat_id } else { chat_id };
    let mut env = Envelope::new(Origin::Interagent, target_chat_id);
    env.topic_id = result.topic_id;
    env.result_text = result.result_text.clone();
    env.status = if result.success { "success".to_string() } else { "error".to_string() };
    env.is_error = !result.success;
    env.lock_mode = if result.success { LockMode::Required } else { LockMode::None };
    
    if result.success && injection_prompt.is_some() {
        env.needs_injection = true;
        env.prompt = injection_prompt.unwrap().to_string();
    }

    env.metadata.insert("sender".to_string(), result.sender.clone());
    env.metadata.insert("recipient".to_string(), result.recipient.clone());
    env.metadata.insert("task_id".to_string(), result.task_id.clone());
    if let Some(ref err) = result.error {
        env.metadata.insert("error".to_string(), err.clone());
    }
    if let Some(t) = transport {
        env.transport = t.to_string();
        env.metadata.insert("transport".to_string(), t.to_string());
    }
    env
}

pub fn from_task_result(result: &TaskResult) -> Envelope {
    let mut env = Envelope::new(Origin::TaskResult, result.chat_id);
    env.topic_id = result.thread_id;
    env.status = result.status.clone();
    env.is_error = result.status == "failed";
    env.lock_mode = if result.status == "cancelled" { LockMode::None } else { LockMode::Required };
    env.needs_injection = result.status != "cancelled";
    
    if env.needs_injection {
        env.prompt = if result.status == "done" {
            format!(
                "[BACKGROUND TASK COMPLETED]\ntask_id='{}'\nresult:\n{}\nReview this result critically.",
                result.task_id, result.result_text
            )
        } else {
            format!(
                "[BACKGROUND TASK FAILED]\ntask_id='{}'\nerror:\n{}\nReview this failure.",
                result.task_id, result.error
            )
        };
    }

    env.elapsed_seconds = result.elapsed_seconds;
    env.provider = result.provider.clone();
    env.model = result.model.clone();
    env.session_id = result.session_id.clone();

    env.metadata.insert("name".to_string(), result.name.clone());
    env.metadata.insert("parent_agent".to_string(), result.parent_agent.clone());
    if !result.error.is_empty() {
        env.metadata.insert("error".to_string(), result.error.clone());
    }
    env
}

pub fn from_task_question(
    task_id: &str,
    question: &str,
    preview: &str,
    chat_id: i64,
    topic_id: Option<i64>,
) -> Envelope {
    let mut env = Envelope::new(Origin::TaskQuestion, chat_id);
    env.topic_id = topic_id;
    env.prompt = question.to_string();
    env.prompt_preview = preview.to_string();
    env.lock_mode = LockMode::Required;
    env.needs_injection = true;
    env.metadata.insert("task_id".to_string(), task_id.to_string());
    env
}

pub fn from_user_message(
    chat_id: i64,
    text: &str,
    topic_id: Option<i64>,
    source: Option<Origin>,
) -> Envelope {
    let mut env = Envelope::new(source.unwrap_or(Origin::User), chat_id);
    env.topic_id = topic_id;
    env.prompt = text.to_string();
    env.prompt_preview = text.chars().take(80).collect();
    env
}
