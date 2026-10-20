//! # Command Line Agent Providers (index.md)
//!
//! ## Overview
//! Wraps and controls executing agent CLI executables (`agy`).
//!
//! ## Submodules
//! - [`antigravity`] (`antigravity/mod.rs`): Spawns PTYs, monitors transcripts, and manages active CLI sessions.
//!
//! ## Search Tags
//! #cli-provider, #agent-executor, #pty-terminal

pub mod antigravity;

use async_trait::async_trait;
use futures::stream::BoxStream;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize)]
/// Represents `CliResponse` (cli response).
pub struct CliResponse {
    /// Session id.
    pub session_id: Option<String>,
    /// Result.
    pub result: String,
    /// Is error.
    pub is_error: bool,
    /// Returncode.
    pub returncode: Option<i32>,
    /// Stderr.
    pub stderr: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
/// Represents `AskQuestionData` (ask question data).
pub struct AskQuestionData {
    /// Question.
    pub question: String,
    /// Options.
    pub options: Vec<String>,
    /// Is multi select.
    pub is_multi_select: bool,
}

#[derive(Clone, Debug)]
/// Enumerates `StreamEvent` (stream event) variants.
pub enum StreamEvent {
    /// `TextDelta` (text delta) variant.
    TextDelta(String),
    /// `Result` (result) variant.
    Result(CliResponse),
    /// `AskQuestion` (ask question) variant.
    AskQuestion(Vec<AskQuestionData>),
}

#[async_trait]
/// Defines the `AgentProvider` (agent provider) interface.
pub trait AgentProvider: Send + Sync {
    /// Executes `send` (send).
    async fn send(
        &self,
        prompt: &str,
        resume_session: Option<&str>,
        continue_session: bool,
        workspace: PathBuf,
    ) -> Result<CliResponse, String>;

    /// Executes `send_streaming` (send streaming).
    async fn send_streaming<'a>(
        &'a self,
        prompt: &str,
        resume_session: Option<&str>,
        continue_session: bool,
        workspace: PathBuf,
    ) -> Result<BoxStream<'a, StreamEvent>, String>;
}
