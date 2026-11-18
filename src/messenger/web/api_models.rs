//! # Web Dashboard Model & Slash Command Catalog (`api_models.rs`)
//!
//! ## Overview
//! Queries live CLI model discovery catalogs, groups variants with supported reasoning
//! effort levels, and exposes provider-specific slash command metadata for `/dashboard/api/models`.
//!
//! ## Search Tags
//! #model-catalog, #effort-levels, #slash-commands, #provider-models

use super::auth_http::require_auth;
use super::{resolve_ctx, WebAppCtx};
use crate::messenger::telegram::callbacks_effort::{
    discover_models_with_default, group_discovered_models,
};
use crate::messenger::telegram::commands_registry::{
    workflow_commands_for_provider, COMMON_COMMANDS,
};
use crate::messenger::telegram::reply_directives::session_supports_effort;
use crate::session::data::SessionData;
use axum::{
    extract::Query,
    http::HeaderMap,
    response::{IntoResponse, Response},
    Extension, Json,
};
use serde::Deserialize;
use std::sync::Arc;

/// Query parameters for `GET /dashboard/api/models`.
#[derive(Deserialize)]
pub struct ModelsQuery {
    /// Provider name (`antigravity`, `claude`, `codex`).
    pub provider: Option<String>,
}

fn build_slash_catalog(prov: &str) -> Vec<serde_json::Value> {
    let mut cmds = Vec::new();
    let skip_web = ["dashboard", "remote", "upgrade", "restart"];
    for &(cmd, desc) in COMMON_COMMANDS {
        if skip_web.contains(&cmd) {
            continue;
        }
        let kind = if cmd == "compact" { "workflow" } else { "builtin" };
        cmds.push(serde_json::json!({ "command": cmd, "description": desc, "kind": kind }));
    }
    cmds.push(serde_json::json!({
        "command": "auth",
        "description": "Open in-dashboard remote browser for login / 2FA / HITL handoff",
        "kind": "builtin"
    }));
    cmds.push(serde_json::json!({
        "command": "help",
        "description": "Show available slash commands and shortcuts",
        "kind": "builtin"
    }));
    for &(cmd, desc) in workflow_commands_for_provider(prov) {
        if !cmds.iter().any(|c| c.get("command").and_then(|v| v.as_str()) == Some(cmd)) {
            cmds.push(serde_json::json!({ "command": cmd, "description": desc, "kind": "workflow" }));
        }
    }
    cmds
}

/// Lists discovered models, effort levels, and slash commands (`GET /dashboard/api/models`).
pub async fn handle_list_models(
    ext: Option<Extension<Arc<WebAppCtx>>>,
    headers: HeaderMap,
    Query(q): Query<ModelsQuery>,
) -> Response {
    let ctx = match resolve_ctx(ext) {
        Ok(c) => c,
        Err(r) => return *r,
    };
    if let Err(r) = require_auth(&ctx, &headers).await {
        return *r;
    }
    let prov = SessionData::effective_provider(q.provider.as_deref().unwrap_or(""), &ctx.config.provider);
    let raw_models = discover_models_with_default(&ctx.cli, prov).await;
    let grouped = group_discovered_models(&raw_models);
    let entries: Vec<serde_json::Value> = grouped
        .into_iter()
        .map(|(model, mut efforts)| {
            if efforts.is_empty() && session_supports_effort(prov, &model) {
                efforts = match prov {
                    "claude" => ["low", "medium", "high", "xhigh", "max"].map(str::to_string).to_vec(),
                    "codex" => ["minimal", "low", "medium", "high", "xhigh"].map(str::to_string).to_vec(),
                    _ => crate::cli::antigravity::AntigravityCli::supported_efforts_for_model(&model),
                };
            }
            serde_json::json!({ "model": model, "efforts": efforts })
        })
        .collect();
    let commands = build_slash_catalog(prov);
    Json(serde_json::json!({ "provider": prov, "models": entries, "commands": commands })).into_response()
}
