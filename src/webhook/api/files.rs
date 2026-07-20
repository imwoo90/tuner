//! # File Transfers and Helpers
//!
//! Handles file sanitization, path verification, downloading, and uploading.

//! 
//! ## Search Tags
//! #files

use crate::webhook::api::server::ApiServerState;
use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode, header},
    response::IntoResponse,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub fn sanitize_filename(name: &str) -> String {
    static SANITIZE_RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = SANITIZE_RE.get_or_init(|| regex::Regex::new(r#"[\\/<>:"|?*\x00]"#).unwrap());
    let mut cleaned = re.replace_all(name, "_").into_owned();
    while cleaned.contains("__") {
        cleaned = cleaned.replace("__", "_");
    }
    let trimmed = cleaned.trim_matches(|c| c == '_' || c == '.' || c == ' ');
    if trimmed.is_empty() {
        "file".to_string()
    } else {
        let limited: String = trimmed.chars().take(120).collect();
        limited
    }
}

pub fn prepare_destination(base_dir: &Path, file_name: &str) -> PathBuf {
    let now_str = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let day_dir = base_dir.join(now_str);
    let _ = std::fs::create_dir_all(&day_dir);
    let mut dest = day_dir.join(file_name);
    if dest.exists() {
        let path = Path::new(file_name);
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        let suffix = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|ext| format!(".{}", ext))
            .unwrap_or_default();
        let mut counter = 1;
        while dest.exists() {
            dest = day_dir.join(format!("{}_{}{}", stem, counter, suffix));
            counter += 1;
        }
    }
    dest
}

pub fn is_image_path(path_str: &str) -> bool {
    let path = Path::new(path_str);
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        let ext_lower = ext.to_lowercase();
        match ext_lower.as_str() {
            "bmp" | "gif" | "jpg" | "jpeg" | "png" | "webp" => true,
            _ => false,
        }
    } else {
        false
    }
}

pub fn parse_file_refs(text: &str) -> Vec<serde_json::Value> {
    static REFS_RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = REFS_RE.get_or_init(|| regex::Regex::new(r"<file:([^>]+)>").unwrap());
    let mut refs = Vec::new();
    for cap in re.captures_iter(text) {
        let raw_path = &cap[1];
        let mut path_str = raw_path.replace('\\', "/");
        if path_str.starts_with('/')
            && path_str.len() >= 3
            && path_str.chars().nth(1).unwrap().is_alphabetic()
            && path_str.chars().nth(2) == Some('/')
        {
            let letter = path_str.chars().nth(1).unwrap();
            let rest: String = path_str.chars().skip(2).collect();
            path_str = format!("{}:{}", letter, rest);
        }
        let path_obj = Path::new(&path_str);
        let name = path_obj
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        refs.push(serde_json::json!({
            "path": path_str,
            "name": name,
            "is_image": is_image_path(&path_str),
        }));
    }
    refs
}

pub fn verify_bearer(headers: &HeaderMap, expected_token: &str) -> bool {
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    crate::webhook::auth::validate_bearer_token(auth, expected_token)
}

pub async fn handle_file_download(
    State(state): State<Arc<std::sync::Mutex<ApiServerState>>>,
    headers: HeaderMap,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    let token = { state.lock().unwrap().config.token.clone() };
    if !verify_bearer(&headers, &token) {
        return (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
    }
    let Some(raw_path) = params.get("path") else {
        return (StatusCode::BAD_REQUEST, "missing 'path' query parameter").into_response();
    };
    let file_path = PathBuf::from(raw_path);
    let roots = {
        let s = state.lock().unwrap();
        match &s.allowed_roots {
            Some(r) if !r.is_empty() => r.clone(),
            _ => {
                let mut fallback = Vec::new();
                if let Some(ws) = &s.workspace {
                    fallback.push(ws.clone());
                } else if let Ok(pwd) = std::env::current_dir() {
                    fallback.push(pwd);
                } else {
                    fallback.push(PathBuf::from("."));
                }
                fallback
            }
        }
    };
    if !crate::security::paths::is_path_safe(&file_path, &roots) {
        return (StatusCode::FORBIDDEN, "path outside allowed roots").into_response();
    }
    if !file_path.is_file() {
        return (StatusCode::NOT_FOUND, "file not found").into_response();
    }
    let mime = mime_guess::from_path(&file_path).first_or_octet_stream();
    match tokio::fs::read(&file_path).await {
        Ok(bytes) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, mime.to_string())],
            bytes,
        )
            .into_response(),
        Err(_) => (StatusCode::NOT_FOUND, "file not found").into_response(),
    }
}

struct UploadedFile {
    file_bytes: Vec<u8>,
    file_name: String,
    caption: Option<String>,
    mime_type: String,
}

async fn parse_multipart(mut multipart: axum::extract::Multipart) -> Result<UploadedFile, String> {
    let mut file_bytes = Vec::new();
    let mut file_name = String::new();
    let mut caption = None;
    let mut mime_type = "application/octet-stream".to_string();

    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.name().unwrap_or("").to_string();
        if name == "file" {
            file_name = field.file_name().unwrap_or("upload").to_string();
            mime_type = field
                .content_type()
                .unwrap_or("application/octet-stream")
                .to_string();
            if let Ok(bytes) = field.bytes().await {
                file_bytes = bytes.to_vec();
            }
        } else if name == "caption" {
            if let Ok(text) = field.text().await {
                caption = Some(text);
            }
        }
    }

    if file_bytes.is_empty() {
        return Err("expected a 'file' field".to_string());
    }
    Ok(UploadedFile {
        file_bytes,
        file_name,
        caption,
        mime_type,
    })
}

pub async fn handle_file_upload(
    State(state): State<Arc<std::sync::Mutex<ApiServerState>>>,
    headers: HeaderMap,
    multipart: axum::extract::Multipart,
) -> impl IntoResponse {
    let token = { state.lock().unwrap().config.token.clone() };
    if !verify_bearer(&headers, &token) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": "unauthorized" })),
        )
            .into_response();
    }

    let (upload_dir, _workspace) = {
        let s = state.lock().unwrap();
        (s.upload_dir.clone(), s.workspace.clone())
    };
    let Some(upload_dir) = upload_dir else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({ "error": "file uploads not configured" })),
        )
            .into_response();
    };

    let uploaded = match parse_multipart(multipart).await {
        Ok(u) => u,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": e })),
            )
                .into_response();
        }
    };

    if uploaded.file_bytes.len() > 50 * 1024 * 1024 {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(serde_json::json!({ "error": "file exceeds 50 MB limit" })),
        )
            .into_response();
    }

    let safe_name = sanitize_filename(&uploaded.file_name);
    let dest = prepare_destination(&upload_dir, &safe_name);

    if let Err(e) = tokio::fs::write(&dest, &uploaded.file_bytes).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response();
    }

    let resp = build_upload_response(
        &dest,
        &uploaded.mime_type,
        uploaded.file_bytes.len(),
        uploaded.caption.as_deref(),
    );
    (StatusCode::OK, Json(resp)).into_response()
}

fn build_upload_response(
    dest: &Path,
    mime_type: &str,
    size: usize,
    caption: Option<&str>,
) -> serde_json::Value {
    let name = dest.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let mut prompt = format!(
        "[INCOMING FILE]\nname='{}'\npath='{}'\ntype='{}'\nvia API",
        name,
        dest.to_string_lossy(),
        mime_type
    );
    if let Some(cap) = caption {
        prompt = format!("{}\nCaption: {}", prompt, cap);
    }
    serde_json::json!({
        "path": dest.to_string_lossy().to_string(),
        "name": name,
        "mime": mime_type,
        "size": size,
        "prompt": prompt,
    })
}
