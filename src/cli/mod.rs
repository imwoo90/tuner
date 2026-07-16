//! # CLI Providers Module
//!
//! This module defines the abstraction layer for integrating various AI Agent CLI tools.
//! It includes the core trait [`AgentProvider`] and data types [`CliResponse`] and [`StreamEvent`].

pub mod antigravity;

use async_trait::async_trait;
use futures::stream::BoxStream;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CliResponse {
    pub session_id: Option<String>,
    pub result: String,
    pub is_error: bool,
    pub returncode: Option<i32>,
    pub stderr: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AskQuestionData {
    pub question: String,
    pub options: Vec<String>,
    pub is_multi_select: bool,
}

#[derive(Clone, Debug)]
pub enum StreamEvent {
    TextDelta(String),
    Result(CliResponse),
    AskQuestion(AskQuestionData),
}

#[async_trait]
pub trait AgentProvider: Send + Sync {
    async fn send(
        &self,
        prompt: &str,
        resume_session: Option<&str>,
        continue_session: bool,
        workspace: PathBuf,
    ) -> Result<CliResponse, String>;

    async fn send_streaming<'a>(
        &'a self,
        prompt: &str,
        resume_session: Option<&str>,
        continue_session: bool,
        workspace: PathBuf,
    ) -> Result<BoxStream<'a, StreamEvent>, String>;
}
