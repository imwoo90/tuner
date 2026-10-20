//! # Background Execution Models
//!
//! ## Overview
//! Defines parameter structures for registering background tasks and tracking output payloads.
//!
//! ## Search Tags
//! #task-models, #task-submit, #execution-result

#[derive(Clone, Debug)]
/// Represents `BackgroundSubmit` (background submit).
pub struct BackgroundSubmit {
    /// Chat id.
    pub chat_id: i64,
    /// Prompt.
    pub prompt: String,
    /// Message id.
    pub message_id: i64,
    /// Thread id.
    pub thread_id: Option<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
/// Enumerates `BackgroundResultStatus` (background result status) variants.
pub enum BackgroundResultStatus {
    #[serde(rename = "ok")]
    /// `Success` (success) variant.
    Success,
    #[serde(rename = "aborted")]
    /// `Aborted` (aborted) variant.
    Aborted,
    #[serde(rename = "error:cli")]
    /// `ErrorCli` (error cli) variant.
    ErrorCli,
    #[serde(rename = "error:timeout")]
    /// `ErrorTimeout` (error timeout) variant.
    ErrorTimeout,
    #[serde(rename = "error:cli_not_found")]
    /// `ErrorCliNotFound` (error cli not found) variant.
    ErrorCliNotFound,
    #[serde(rename = "error:internal")]
    /// `ErrorInternal` (error internal) variant.
    ErrorInternal,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
/// Represents `BackgroundResult` (background result).
pub struct BackgroundResult {
    /// Task id.
    pub task_id: String,
    /// Chat id.
    pub chat_id: i64,
    /// Message id.
    pub message_id: i64,
    /// Thread id.
    pub thread_id: Option<i64>,
    /// Prompt preview.
    pub prompt_preview: String,
    /// Result text.
    pub result_text: String,
    /// Status.
    pub status: BackgroundResultStatus,
    /// Elapsed seconds.
    pub elapsed_seconds: f64,
}
