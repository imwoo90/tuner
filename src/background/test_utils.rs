//! # Test Harness and Mocks for Background Module
//!
//! ## Overview
//! Provides fake task providers, command runners, and callback catchers to simulate asynchronous
//! agent runs during integration testing.
//!
//! ## Search Tags
//! #background-tests, #mock-provider, #test-harness
#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::background::observer::BackgroundSubmit;
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
        panic!("send_streaming not used in observer tests");
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
