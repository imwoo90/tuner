//! # OpenAI Codex CLI Provider (mod.rs)
//!
//! ## Overview
//! Drives OpenAI Codex via a persistent warm `codex app-server --stdio` JSON-RPC process per
//! conversation topic, parsing thread IDs, reasoning items, command executions, and streaming
//! agent deltas into `StreamEvent` items.
//!
//! ## Search Tags
//! #codex-provider, #app-server-stdio, #warm-process, #agent-executor
//!
//! ## Submodules
//! - [`parser`] (`parser.rs`): Incremental JSON-RPC and JSONL stream parser.
//! - [`session`] (`session.rs`): Persistent warm `codex app-server --stdio` session pool and turn signaling.
//! - [`spawner`] (`spawner.rs`): Child process group spawner and JSON-RPC `initialize`/`thread` handshake.
//! - [`runner`] (`runner.rs`): Asynchronous JSON-RPC `turn/start` executor and stream finalizer.

pub mod parser;
pub mod runner;
pub mod session;
pub mod spawner;

use crate::cli::antigravity::AntigravityCli;
pub use crate::cli::common::parse_codex_models_response;
use crate::cli::common::{probe_cli_json_lines, resolve_cli_bin, strip_effort_suffix};
pub use parser::CodexStreamParser;
pub use runner::{send, send_streaming};
pub use session::{
    active_count, active_pid, active_session_id, cleanup_expired, has_pending_ask,
    interrupt_session, is_running, submit_ask_answers, terminate_all, terminate_all_for_chat,
    terminate_topic,
};

/// Fallback OpenAI Codex model identifiers when `codex app-server` cannot be probed.
pub const CODEX_MODELS: &[&str] = &[
    "gpt-6.1-sol", "gpt-6-astra", "gpt-6-sol", "gpt-6-luna", "gpt-5.6-sol",
    "gpt-6.1-terra", "gpt-5.6-terra", "gpt-5.6-luna", "gpt-5.5",
];

/// Default model when switching to the `codex` provider.
pub const DEFAULT_CODEX_MODEL: &str = "gpt-6-astra";

/// Supported reasoning effort levels for the `codex` CLI.
pub const CODEX_EFFORTS: &[&str] = &["minimal", "low", "medium", "high", "xhigh"];

/// Returns the static fallback model list (with effort suffixes) for the `codex` provider.
pub fn discover_models() -> Vec<String> {
    CODEX_MODELS.iter().flat_map(|b| CODEX_EFFORTS.iter().map(move |e| format!("{}-{}", b, e))).collect()
}

/// Dynamically queries `codex app-server --stdio` `model/list` for the user's live `/model` options.
pub async fn discover_models_live(env: &std::collections::HashMap<String, String>) -> Vec<String> {
    let init = format!(
        r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"clientInfo":{{"name":"tuner","version":"{}"}}}}}}"#,
        env!("CARGO_PKG_VERSION")
    );
    let reqs = [
        init.as_str(),
        r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#,
        r#"{"jsonrpc":"2.0","id":2,"method":"model/list","params":{}}"#,
    ];
    probe_cli_json_lines(&resolve_codex_bin(), &["app-server", "--stdio"], &reqs, env, 10)
        .await.iter().find_map(|l| Some(parse_codex_models_response(l)).filter(|p| !p.is_empty()))
        .unwrap_or_else(discover_models)
}

/// Resolves the `codex` binary path from `PATH` or user local bin fallback.
pub fn resolve_codex_bin() -> String { resolve_cli_bin("codex") }

fn normalize_codex_model(raw: &str) -> &str {
    match raw {
        "astra" => "gpt-6-astra",
        "sol" => "gpt-6.1-sol",
        "terra" => "gpt-6.1-terra",
        "luna" => "gpt-6-luna",
        other => other,
    }
}

/// Returns `true` when the given Codex model supports reasoning effort configuration.
pub fn supports_codex_effort(model: &str) -> bool {
    let low = model.trim().to_ascii_lowercase();
    !low.is_empty() && low != "antigravity-default"
}

/// Resolves the canonical model and reasoning effort flags for the `codex` CLI.
pub fn resolve_codex_model_effort(model: Option<&str>, effort: Option<&str>) -> (Option<String>, Option<String>) {
    let mut suffix_eff = None;
    let explicit_effortless = model.map(str::trim).is_some_and(|m| !m.is_empty() && !supports_codex_effort(m));
    let resolved_model = model
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("antigravity-default"))
        .and_then(|raw| {
            let low = raw.to_ascii_lowercase();
            let (base, eff) = strip_effort_suffix(&low);
            let c = normalize_codex_model(base);
            let ok = crate::cli::codex::CODEX_MODELS.contains(&c)
                || ((c.starts_with("gpt-5")
                    || c.starts_with("gpt-6")
                    || ["o1", "o3", "o4", "codex"].iter().any(|p| c.starts_with(p)))
                    && !c.starts_with("gpt-oss"));
            if ok {
                suffix_eff = eff.map(str::to_string);
                Some(c.to_string())
            } else {
                None
            }
        });
    if explicit_effortless {
        return (resolved_model, None);
    }
    let raw_eff = effort.map(str::trim).filter(|e| !e.is_empty()).map(str::to_lowercase).or(suffix_eff);
    let resolved_eff = raw_eff.and_then(|e| match e.as_str() {
        "minimal" | "low" | "medium" | "high" => Some(e),
        "xhigh" | "max" | "ultra" => Some("xhigh".to_string()),
        _ => None,
    });
    (resolved_model, resolved_eff)
}

/// Builds the persistent warm `codex app-server --stdio` command-line vector.
pub fn build_app_server_command(cli: &AntigravityCli) -> Vec<String> {
    let mut cmd = [
        resolve_codex_bin().as_str(), "app-server", "--stdio",
        "-c", "features.default_mode_request_user_input=true",
        "-c", "suppress_unstable_features_warning=true",
    ].into_iter().map(str::to_string).collect::<Vec<_>>();
    if let Some(params) = cli.config.cli_parameters.get("codex") {
        let mut iter = params.iter();
        while let Some(a) = iter.next() {
            if matches!(a.as_str(), "-c" | "--config" | "--enable" | "--disable") {
                if let Some(v) = iter.next() { cmd.extend([a.clone(), v.clone()]); }
            } else if a.starts_with("-c=") || a.starts_with("--config=") {
                cmd.push(a.clone());
            }
        }
    }
    cmd
}

/// Builds the command-line invocation vector for a `codex` CLI turn.
pub fn build_command(
    cli: &AntigravityCli,
    prompt: &str,
    resume_session: Option<&str>,
    continue_session: bool,
) -> Vec<String> {
    let mut cmd = vec![resolve_codex_bin(), "exec".to_string()];
    if let Some(sid) = resume_session.filter(|s| !s.is_empty() && !s.starts_with("mock-")) {
        cmd.extend(["resume".to_string(), sid.to_string()]);
    } else if continue_session {
        cmd.extend(["resume".to_string(), "--last".to_string()]);
    }
    cmd.extend(["--json".to_string(), "--skip-git-repo-check".to_string()]);
    if cli.config.permission_mode == "bypassPermissions" {
        cmd.push("--dangerously-bypass-approvals-and-sandbox".to_string());
    }
    let (m, eff) = resolve_codex_model_effort(cli.config.model.as_deref(), cli.config.effort.as_deref());
    if let Some(m) = m { cmd.extend(["-m".to_string(), m]); }
    if let Some(e) = eff { cmd.extend(["-c".to_string(), format!("model_reasoning_effort=\"{}\"", e)]); }
    if let Some(p) = cli.config.cli_parameters.get("codex") { cmd.extend(p.iter().cloned()); }
    cmd.push(cli.format_prompt(prompt));
    cmd
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod mod_tests;
