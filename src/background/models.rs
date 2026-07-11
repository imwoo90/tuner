//! # Background Models
//!
//! Type definitions for background execution submissions and results.

#[derive(Clone, Debug)]
pub struct BackgroundSubmit {
    pub chat_id: i64,
    pub prompt: String,
    pub message_id: i64,
    pub thread_id: Option<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum BackgroundResultStatus {
    #[serde(rename = "ok")]
    Success,
    #[serde(rename = "aborted")]
    Aborted,
    #[serde(rename = "error:cli")]
    ErrorCli,
    #[serde(rename = "error:timeout")]
    ErrorTimeout,
    #[serde(rename = "error:cli_not_found")]
    ErrorCliNotFound,
    #[serde(rename = "error:internal")]
    ErrorInternal,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct BackgroundResult {
    pub task_id: String,
    pub chat_id: i64,
    pub message_id: i64,
    pub thread_id: Option<i64>,
    pub prompt_preview: String,
    pub result_text: String,
    pub status: BackgroundResultStatus,
    pub elapsed_seconds: f64,
}
