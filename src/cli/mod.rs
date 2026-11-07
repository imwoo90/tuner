//! # Command Line Agent Providers (index.md)
//!
//! ## Overview
//! Orchestrates the 1:1:1 peer multi-provider CLI engines (`antigravity`, `claude`, `codex`)
//! and shared formatting/process utilities (`common`).
//!
//! ## Submodules
//! - [`antigravity`] (`antigravity/mod.rs`): Google Antigravity (`agy`) PTY + JSONL transcript driver.
//! - [`claude`] (`claude/mod.rs`): Anthropic Claude Code (`claude`) persistent warm `stream-json` driver.
//! - [`codex`] (`codex/mod.rs`): OpenAI Codex (`codex app-server --stdio`) persistent warm JSON-RPC driver.
//! - [`common`] (`common.rs`): Shared effort suffix parsing, collapsible stream preview formatting, and process group helpers.
//! - [`peer`] (`peer.rs`): Peer session existence checks, workspace auto-trust, command discovery, and ask-question parsers.
//!
//! ## Search Tags
//! #cli-provider, #agent-executor, #multi-provider, #warm-process

pub mod antigravity;
pub mod claude;
pub mod codex;
pub mod common;
pub mod peer;

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
