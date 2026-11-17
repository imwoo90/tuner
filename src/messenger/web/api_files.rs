//! # Sandboxed Workspace File Preview & Zero-Knowledge Vault API (`api_files.rs`)
//!
//! ## Overview
//! Enforces strict workspace canonicalization and sensitive-path blocking on `/dashboard/api/file`
//! (both JSON metadata/text previews and raw media/binary streaming) and manages write-only
//! credential storage for `/dashboard/api/vault/entries`.
//!
//! ## Search Tags
//! #file-sandbox, #split-pane-preview, #vault-api, #path-traversal-guard

use super::auth_http::{json_err, require_auth, require_mutation_auth};
use super::{resolve_ctx, WebAppCtx};
use crate::cli::antigravity::events::agy_state_root;
use crate::messenger::telegram::attachments::extract_file_paths;
use crate::messenger::telegram::review::detect_language;
use crate::messenger::telegram::review_store::inspect_zip_entries;
use crate::security::paths::{resolve_path, validate_file_path};
use crate::security::vault::{VaultEntry, VaultStore};
use axum::{
    extract::{Path as AxumPath, Query, Request},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Extension, Json,
};
use serde::Deserialize;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use tower::ServiceExt;

const MAX_PREVIEW_BYTES: usize = 512 * 1024;

fn build_file_allowed_roots(ctx: &WebAppCtx) -> Vec<PathBuf> {
    let mut roots = vec![
        ctx.working_dir.clone(),
        ctx.tuner_home.clone(),
        agy_state_root(Some(&ctx.cli.build_env())).join("brain"),
    ];
    if let Some(prof_home) = ctx.vault_dir.parent() {
        roots.push(prof_home.to_path_buf());
    }
    roots
}

fn is_blocked_filename(low: &str, prev_comp: &str) -> bool {
    low == ".env"
        || low.starts_with(".env.")
        || matches!(
            low,
            "dashboard_auth.json"
                | ".dashboard_auth.lock"
                | "tunnel_endpoint.json"
                | "dashboard_tunnel.json"
                | "vault.enc"
                | "vault.key"
                | "antigravity-oauth-token"
                | ".credentials.json"
                | "auth.json"
        )
        || (low == "config.json" && prev_comp.eq_ignore_ascii_case("config"))
}

/// Returns `true` if `path` touches any secret directory or protected credential file.
pub fn is_sensitive_path(path: &Path, vault_dir: &Path) -> bool {
    if let (Ok(r_p), Ok(r_v)) = (resolve_path(path), resolve_path(vault_dir))
        && r_p.starts_with(&r_v)
    {
        return true;
    }
    let mut prev_comp = String::new();
    for comp in path.components() {
        if let Component::Normal(os) = comp {
            let low = os.to_string_lossy().to_ascii_lowercase();
            if matches!(low.as_str(), "vault" | ".ssh" | ".gnupg" | ".aws")
                || is_blocked_filename(&low, &prev_comp)
            {
                return true;
            }
            prev_comp = low;
        }
    }
    false
}

/// Extracts safe, non-sensitive workspace file paths from markdown/text replies.
pub fn filter_safe_preview_paths(ctx: &WebAppCtx, text: &str) -> Vec<String> {
    let roots = build_file_allowed_roots(ctx);
    extract_file_paths(text, &roots)
        .into_iter()
        .filter(|p| !is_sensitive_path(p, &ctx.vault_dir))
        .map(|p| p.to_string_lossy().to_string())
        .collect()
}

/// Validates that `raw_path` canonicalizes inside allowed roots and is not sensitive.
pub fn validate_preview_file(ctx: &WebAppCtx, raw_path: &str) -> Result<PathBuf, (StatusCode, &'static str)> {
    let trimmed = raw_path.trim();
    if trimmed.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "Missing path parameter"));
    }
    let raw = Path::new(trimmed);
    if raw.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err((StatusCode::FORBIDDEN, "Parent directory traversal is forbidden"));
    }
    if is_sensitive_path(raw, &ctx.vault_dir) {
        return Err((StatusCode::FORBIDDEN, "Access denied to sensitive path"));
    }
    let roots = build_file_allowed_roots(ctx);
    let candidate = if raw.is_absolute() { raw.to_path_buf() } else { ctx.working_dir.join(raw) };
    let canon = validate_file_path(&candidate, &roots)
        .map_err(|_| (StatusCode::FORBIDDEN, "Path outside allowed workspace roots"))?;
    if is_sensitive_path(&canon, &ctx.vault_dir) {
        return Err((StatusCode::FORBIDDEN, "Access denied to sensitive path"));
    }
    if !canon.is_file() {
        return Err((StatusCode::NOT_FOUND, "File not found"));
    }
    Ok(canon)
}

/// Query parameters for `GET /dashboard/api/file`.
#[derive(Deserialize)]
pub struct FileQuery {
    /// Absolute or workspace-relative file path.
    pub path: String,
    /// Set `raw=1` for binary/raw streaming; `0` (default) for JSON preview metadata/text.
    #[serde(default)]
    pub raw: u8,
}

/// Serves sandboxed text/JSON preview or raw binary streaming (`GET /dashboard/api/file`).
pub async fn handle_get_file(
    ext: Option<Extension<Arc<WebAppCtx>>>,
    headers: HeaderMap,
    Query(q): Query<FileQuery>,
    req: Request,
) -> Response {
    let ctx = match resolve_ctx(ext) {
        Ok(c) => c,
        Err(r) => return *r,
    };
    if let Err(r) = require_auth(&ctx, &headers).await {
        return *r;
    }
    let canon = match validate_preview_file(&ctx, &q.path) {
        Ok(p) => p,
        Err((code, msg)) => return json_err(code, msg),
    };
    if q.raw == 1 {
        return serve_raw_file(&canon, req).await;
    }
    build_file_preview_json(&canon)
}

async fn serve_raw_file(canon: &Path, req: Request) -> Response {
    let ext_low = canon
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();
    let force_download = matches!(ext_low.as_str(), "html" | "htm" | "xhtml" | "xml" | "svg");
    let serve = tower_http::services::ServeFile::new(canon);
    match serve.oneshot(req).await {
        Ok(res) => {
            let mut out = res.into_response();
            if let Ok(hv) = header::HeaderValue::from_str("nosniff") {
                out.headers_mut().insert(header::X_CONTENT_TYPE_OPTIONS, hv);
            }
            if let Ok(hv) = header::HeaderValue::from_str(
                "sandbox; default-src 'none'; img-src 'self' data:; media-src 'self'; style-src 'unsafe-inline'",
            ) {
                out.headers_mut().insert(header::CONTENT_SECURITY_POLICY, hv);
            }
            if force_download {
                let fname = canon
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("download")
                    .replace('"', "_");
                if let Ok(hv) = header::HeaderValue::from_str(&format!("attachment; filename=\"{fname}\"")) {
                    out.headers_mut().insert(header::CONTENT_DISPOSITION, hv);
                }
            }
            out
        }
        Err(e) => json_err(StatusCode::INTERNAL_SERVER_ERROR, &format!("File error: {e}")),
    }
}

fn preview_json_response(
    canon: &Path,
    language: &str,
    size_bytes: usize,
    is_binary: bool,
    truncated: bool,
    content: &str,
) -> Response {
    let filename = canon.file_name().and_then(|n| n.to_str()).unwrap_or("file");
    Json(serde_json::json!({
        "path": canon.to_string_lossy(),
        "filename": filename,
        "language": language,
        "size_bytes": size_bytes,
        "is_binary": is_binary,
        "truncated": truncated,
        "content": content
    }))
    .into_response()
}

fn build_file_preview_json(canon: &Path) -> Response {
    let language = detect_language(canon);
    let size_bytes = std::fs::metadata(canon).map(|m| m.len() as usize).unwrap_or(0);
    if language == "zip" {
        let content = inspect_zip_entries(canon).unwrap_or_default();
        return preview_json_response(canon, &language, size_bytes, true, false, &content);
    }
    if matches!(language.as_str(), "image" | "svg" | "pdf" | "audio" | "video") {
        return preview_json_response(canon, &language, size_bytes, true, false, "");
    }
    let Ok(f) = std::fs::File::open(canon) else {
        return json_err(StatusCode::INTERNAL_SERVER_ERROR, "Failed to open file");
    };
    let mut buf = Vec::new();
    if f.take((MAX_PREVIEW_BYTES + 1) as u64).read_to_end(&mut buf).is_err() {
        return json_err(StatusCode::INTERNAL_SERVER_ERROR, "Failed to read file");
    }
    let truncated = buf.len() > MAX_PREVIEW_BYTES;
    if truncated {
        buf.truncate(MAX_PREVIEW_BYTES);
    }
    if buf.contains(&0) {
        return preview_json_response(canon, "binary", size_bytes, true, false, "");
    }
    let text = String::from_utf8_lossy(&buf);
    preview_json_response(canon, &language, size_bytes, false, truncated, &text)
}

/// Lists masked Zero-Knowledge Vault entries (`GET /dashboard/api/vault/entries`).
pub async fn handle_vault_list(
    ext: Option<Extension<Arc<WebAppCtx>>>,
    headers: HeaderMap,
) -> Response {
    let ctx = match resolve_ctx(ext) {
        Ok(c) => c,
        Err(r) => return *r,
    };
    if let Err(r) = require_auth(&ctx, &headers).await {
        return *r;
    }
    let store = VaultStore::new(&ctx.vault_dir);
    match store.list_masked() {
        Ok(entries) => Json(serde_json::json!({ "profile": ctx.profile, "entries": entries })).into_response(),
        Err(e) => json_err(StatusCode::INTERNAL_SERVER_ERROR, &e),
    }
}

/// Creates or updates a Vault entry (`POST /dashboard/api/vault/entries`), returning masked rows.
pub async fn handle_vault_upsert(
    ext: Option<Extension<Arc<WebAppCtx>>>,
    headers: HeaderMap,
    Json(payload): Json<VaultEntry>,
) -> Response {
    let ctx = match resolve_ctx(ext) {
        Ok(c) => c,
        Err(r) => return *r,
    };
    if let Err(r) = require_mutation_auth(&ctx, &headers).await {
        return *r;
    }
    let store = VaultStore::new(&ctx.vault_dir);
    if let Err(e) = store.upsert_entry(payload) {
        return json_err(StatusCode::BAD_REQUEST, &e);
    }
    match store.list_masked() {
        Ok(entries) => Json(serde_json::json!({ "profile": ctx.profile, "entries": entries })).into_response(),
        Err(e) => json_err(StatusCode::INTERNAL_SERVER_ERROR, &e),
    }
}

/// Deletes a Vault entry by `site_key` (`DELETE /dashboard/api/vault/entries/:site_key`).
pub async fn handle_vault_delete(
    ext: Option<Extension<Arc<WebAppCtx>>>,
    headers: HeaderMap,
    AxumPath(site_key): AxumPath<String>,
) -> Response {
    let ctx = match resolve_ctx(ext) {
        Ok(c) => c,
        Err(r) => return *r,
    };
    if let Err(r) = require_mutation_auth(&ctx, &headers).await {
        return *r;
    }
    let store = VaultStore::new(&ctx.vault_dir);
    if let Err(e) = store.delete_entry(&site_key) {
        return json_err(StatusCode::BAD_REQUEST, &e);
    }
    match store.list_masked() {
        Ok(entries) => Json(serde_json::json!({ "profile": ctx.profile, "entries": entries })).into_response(),
        Err(e) => json_err(StatusCode::INTERNAL_SERVER_ERROR, &e),
    }
}
