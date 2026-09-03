//! # In-App File Review Manager and Web Server
//!
//! Provides ephemeral, secure file snapshot sessions for Telegram WebApp and mobile code review.
//! Automatically captures referenced files, serves a mobile-responsive dark-mode viewer via Axum,
//! and connects to Cloudflare Quick Tunnels for instant external HTTPS access without configuration.

use axum::{
    extract::Path,
    http::StatusCode,
    response::{Html, IntoResponse, Response},
    routing::get,
    Json, Router,
};
use std::collections::HashMap;
use std::path::{Path as StdPath, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ReviewFile {
    pub filename: String,
    pub path: String,
    pub content: String,
    pub size_bytes: usize,
    pub language: String,
}

struct SessionEntry {
    created_at: Instant,
    files: Vec<ReviewFile>,
}

pub struct ReviewManager {
    sessions: Arc<Mutex<HashMap<String, SessionEntry>>>,
    tunnel_url: Arc<Mutex<Option<String>>>,
    server_port: Arc<Mutex<Option<u16>>>,
}

static INSTANCE: OnceLock<Arc<ReviewManager>> = OnceLock::new();

pub fn global_review_manager() -> Arc<ReviewManager> {
    INSTANCE.get_or_init(|| Arc::new(ReviewManager::new())).clone()
}

impl ReviewManager {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
            tunnel_url: Arc::new(Mutex::new(None)),
            server_port: Arc::new(Mutex::new(None)),
        }
    }

    pub async fn create_session(&self, paths: &[PathBuf]) -> Option<(String, usize)> {
        let mut files = Vec::new();
        for path in paths {
            if !path.is_file() {
                continue;
            }
            if let Ok(metadata) = std::fs::metadata(path) {
                if metadata.len() > 1_000_000 {
                    continue;
                }
            }
            if let Ok(content) = std::fs::read_to_string(path) {
                let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("file").to_string();
                let language = detect_language(path);
                files.push(ReviewFile {
                    filename,
                    path: path.to_string_lossy().to_string(),
                    size_bytes: content.len(),
                    content,
                    language,
                });
            }
        }

        if files.is_empty() {
            return None;
        }

        let token = hex::encode(rand::random::<[u8; 16]>());
        let count = files.len();

        let mut map = self.sessions.lock().await;
        let now = Instant::now();
        map.retain(|_, v| now.duration_since(v.created_at) < Duration::from_secs(86400));

        map.insert(token.clone(), SessionEntry {
            created_at: now,
            files,
        });

        Some((token, count))
    }

    pub async fn get_files(&self, token: &str) -> Option<Vec<ReviewFile>> {
        let map = self.sessions.lock().await;
        map.get(token).map(|e| e.files.clone())
    }

    pub async fn get_html(&self, token: &str) -> Option<String> {
        let files = self.get_files(token).await?;
        let json_data = serde_json::to_string(&files).unwrap_or_else(|_| "[]".to_string());
        let template = include_str!("review_viewer.html");
        let rendered = template.replace("__FILES_JSON__", &json_data);
        Some(rendered)
    }

    pub async fn ensure_server_running(&self) -> u16 {
        let mut port_guard = self.server_port.lock().await;
        if let Some(port) = *port_guard {
            return port;
        }

        let listener = match tokio::net::TcpListener::bind("0.0.0.0:8743").await {
            Ok(l) => l,
            Err(_) => tokio::net::TcpListener::bind("0.0.0.0:0").await.expect("Failed to bind ephemeral port"),
        };

        let port = listener.local_addr().map(|a| a.port()).unwrap_or(8743);
        *port_guard = Some(port);

        let app = Router::new()
            .route("/review/:token", get(handle_review_page))
            .route("/review/:token/json", get(handle_review_json))
            .route("/health", get(handle_health));

        tokio::spawn(async move {
            axum::serve(listener, app).await.ok();
        });

        port
    }

    pub async fn get_review_url(&self, token: &str) -> String {
        let port = self.ensure_server_running().await;
        let mut tunnel_guard = self.tunnel_url.lock().await;
        if let Some(ref base) = *tunnel_guard {
            return format!("{}/review/{}", base, token);
        }

        if let Some(url) = spawn_quick_tunnel(port).await {
            *tunnel_guard = Some(url.clone());
            format!("{}/review/{}", url, token)
        } else {
            format!("http://127.0.0.1:{}/review/{}", port, token)
        }
    }
}

fn detect_language(path: &StdPath) -> String {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    match ext.as_str() {
        "rs" => "rust",
        "toml" => "toml",
        "json" | "jsonl" => "json",
        "py" => "python",
        "js" | "mjs" | "cjs" => "javascript",
        "ts" | "tsx" => "typescript",
        "sh" | "bash" => "bash",
        "html" | "htm" => "html",
        "css" => "css",
        "md" | "markdown" => "markdown",
        "yaml" | "yml" => "yaml",
        "sql" => "sql",
        "c" | "h" => "c",
        "cpp" | "hpp" | "cc" => "cpp",
        _ => "plaintext",
    }.to_string()
}

async fn handle_review_page(Path(token): Path<String>) -> Response {
    let mgr = global_review_manager();
    if let Some(html) = mgr.get_html(&token).await {
        Html(html).into_response()
    } else {
        (StatusCode::NOT_FOUND, "Review session not found or expired").into_response()
    }
}

async fn handle_review_json(Path(token): Path<String>) -> Response {
    let mgr = global_review_manager();
    if let Some(files) = mgr.get_files(&token).await {
        Json(files).into_response()
    } else {
        (StatusCode::NOT_FOUND, "Session not found").into_response()
    }
}

async fn handle_health() -> &'static str {
    "OK"
}

async fn spawn_quick_tunnel(port: u16) -> Option<String> {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let bin_candidates = [
        PathBuf::from(&home).join(".tuner/bin/cloudflared"),
        PathBuf::from("/usr/local/bin/cloudflared"),
        PathBuf::from("/usr/bin/cloudflared"),
    ];

    let cloudflared_bin = bin_candidates.iter().find(|p| p.is_file())?.clone();
    let mut cmd = tokio::process::Command::new(cloudflared_bin);
    cmd.args(["tunnel", "--url", &format!("http://127.0.0.1:{}", port)])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let mut child = cmd.spawn().ok()?;
    let stderr = child.stderr.take()?;
    let mut reader = tokio::io::BufReader::new(stderr);
    let mut line = String::new();

    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(10) {
        use tokio::io::AsyncBufReadExt;
        line.clear();
        if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
            break;
        }
        if let Some(pos) = line.find("https://") {
            let sub = &line[pos..];
            if let Some(end) = sub.find(".trycloudflare.com") {
                let url = &sub[..end + ".trycloudflare.com".len()];
                let trimmed = url.trim().to_string();
                tokio::spawn(async move {
                    let _ = child.wait().await;
                });
                return Some(trimmed);
            }
        }
    }
    None
}
