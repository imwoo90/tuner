//! # Background Task Executor Module (index.md)
//!
//! ## Overview
//! Handles fire-and-forget background execution of AI agent provider commands.
//!
//! ## Submodules
//! - [`observer`] (`observer.rs`): Task registry, chat limits, timeout tracking, and process drop guards.
//!
//! ## Search Tags
//! #background-runner, #observer, #async-execution, #task-limits

pub mod observer;

pub use observer::BackgroundObserver;

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

/// Background execution model re-exports.
pub mod models {
    pub use super::{BackgroundResult, BackgroundResultStatus, BackgroundSubmit};
}

/// Test harness and mocks for background module testing.
pub mod test_utils {
    use super::BackgroundSubmit;
    use crate::cli::{AgentProvider, CliResponse, StreamEvent};
    use crate::workspace::paths::DuctorPaths;
    use async_trait::async_trait;
    use futures::stream::BoxStream;
    use std::path::PathBuf;
    use std::sync::Arc;
    use tokio::sync::Notify;

    /// Represents `MockProvider` (mock provider).
    pub struct MockProvider {
        /// Notify.
        pub notify: Option<Arc<Notify>>,
        /// Response.
        pub response: Result<CliResponse, String>,
    }

    #[async_trait]
    impl AgentProvider for MockProvider {
        async fn send(
            &self,
            _prompt: &str,
            _resume_session: Option<&str>,
            _continue_session: bool,
            _workspace: PathBuf,
        ) -> Result<CliResponse, String> {
            if let Some(ref notify) = self.notify {
                notify.notified().await;
            }
            self.response.clone()
        }

        async fn send_streaming<'a>(
            &'a self,
            _prompt: &str,
            _resume_session: Option<&str>,
            _continue_session: bool,
            _workspace: PathBuf,
        ) -> Result<BoxStream<'a, StreamEvent>, String> {
            Err("send_streaming not used in observer tests".to_string())
        }
    }

    /// Executes `make_paths` (make paths).
    pub fn make_paths() -> DuctorPaths {
        DuctorPaths::new(
            PathBuf::from("/tmp/ductor_home"),
            PathBuf::from("/tmp/home_defaults"),
            PathBuf::from("/tmp/framework_root"),
            None,
        )
    }

    /// Executes `make_submit` (make submit).
    pub fn make_submit(chat_id: i64, prompt: &str, message_id: i64) -> BackgroundSubmit {
        BackgroundSubmit {
            chat_id,
            prompt: prompt.to_string(),
            message_id,
            thread_id: None,
        }
    }
}
