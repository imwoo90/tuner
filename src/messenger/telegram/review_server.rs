//! # In-App File Review Web Server
//!
//! Provides the Axum HTTP web server and Cloudflare Quick Tunnel orchestration
//! for file review, code inspection, media streaming (HTTP Range/206), and downloads.

use axum::{
    extract::Path,
    http::{header, StatusCode},
    response::{Html, IntoResponse, Response},
    routing::get,
    Json, Router,
};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tower::ServiceExt;

pub async fn ensure_server_running(server_port: &Arc<Mutex<Option<u16>>>) -> u16 {
    let mut port_guard = server_port.lock().await;
    if let Some(port) = *port_guard {
        if tokio::net::TcpStream::connect(("127.0.0.1", port)).await.is_ok() {
            return port;
        }
        *port_guard = None;
    }

    let bind_addr = if cfg!(test) {
        "0.0.0.0:0"
    } else {
        "0.0.0.0:8743"
    };

    let listener = match tokio::net::TcpListener::bind(bind_addr).await {
        Ok(l) => l,
        Err(_) => tokio::net::TcpListener::bind("0.0.0.0:0")
            .await
            .expect("Failed to bind ephemeral port"),
    };

    let port = listener.local_addr().map(|a| a.port()).unwrap_or(8743);
    *port_guard = Some(port);

    let app = Router::new()
        .route("/review/:token", get(handle_review_page))
        .route("/review/:token/json", get(handle_review_json))
        .route("/review/:token/media/:filename", get(handle_media))
        .route("/review/:token/download/:filename", get(handle_download))
        .route("/health", get(handle_health));

    tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });

    port
}

pub async fn warmup(server_port: &Arc<Mutex<Option<u16>>>, tunnel_url: &Arc<Mutex<Option<String>>>) {
    let port = ensure_server_running(server_port).await;
    let mut tunnel_guard = tunnel_url.lock().await;
    if tunnel_guard.is_none() {
        if let Some(url) = spawn_quick_tunnel(port).await {
            *tunnel_guard = Some(url);
        }
    }
}

pub async fn get_review_url(
    server_port: &Arc<Mutex<Option<u16>>>,
    tunnel_url: &Arc<Mutex<Option<String>>>,
    token: &str,
) -> String {
    let port = ensure_server_running(server_port).await;
    let mut tunnel_guard = tunnel_url.lock().await;
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

fn find_cloudflared_bin() -> Option<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let bin_candidates = [
        PathBuf::from(&home).join(".tuner/bin/cloudflared"),
        PathBuf::from("/usr/local/bin/cloudflared"),
        PathBuf::from("/usr/bin/cloudflared"),
    ];
    bin_candidates.into_iter().find(|p| p.is_file())
}

fn parse_tunnel_url(line: &str) -> Option<String> {
    if line.contains("api.trycloudflare.com") {
        return None;
    }
    let pos = line.find("https://")?;
    let sub = &line[pos..];
    let end = sub.find(".trycloudflare.com")?;
    let url = &sub[..end + ".trycloudflare.com".len()];
    let trimmed = url.trim().to_string();
    if trimmed.contains("api.trycloudflare.com") || trimmed == "https://trycloudflare.com" {
        None
    } else {
        Some(trimmed)
    }
}

pub async fn spawn_quick_tunnel(port: u16) -> Option<String> {
    let bin = find_cloudflared_bin()?;
    let mut cmd = tokio::process::Command::new(bin);
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
        if let Some(url) = parse_tunnel_url(&line) {
            tokio::spawn(async move {
                use tokio::io::AsyncBufReadExt;
                let mut discard = String::new();
                while let Ok(n) = reader.read_line(&mut discard).await {
                    if n == 0 {
                        break;
                    }
                    discard.clear();
                }
                let _ = child.wait().await;
            });
            return Some(url);
        }
    }
    None
}

async fn handle_review_page(Path(token): Path<String>) -> Response {
    let mgr = super::review::global_review_manager();
    if let Some(html) = mgr.get_html(&token).await {
        Html(html).into_response()
    } else {
        (StatusCode::NOT_FOUND, "Review session not found or expired").into_response()
    }
}

async fn handle_review_json(Path(token): Path<String>) -> Response {
    let mgr = super::review::global_review_manager();
    if let Some(files) = mgr.get_files(&token).await {
        Json(files).into_response()
    } else {
        (StatusCode::NOT_FOUND, "Session not found").into_response()
    }
}

async fn handle_media(
    Path((token, filename)): Path<(String, String)>,
    req: axum::extract::Request,
) -> Response {
    let mgr = super::review::global_review_manager();
    let file_path = match mgr.get_file_path(&token, &filename).await {
        Some(p) => p,
        None => return (StatusCode::NOT_FOUND, "File not found").into_response(),
    };

    let serve = tower_http::services::ServeFile::new(&file_path);
    match serve.oneshot(req).await {
        Ok(res) => res.into_response(),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("File error: {}", err),
        )
            .into_response(),
    }
}

async fn handle_download(
    Path((token, filename)): Path<(String, String)>,
    req: axum::extract::Request,
) -> Response {
    let mgr = super::review::global_review_manager();
    let file_path = match mgr.get_file_path(&token, &filename).await {
        Some(p) => p,
        None => return (StatusCode::NOT_FOUND, "File not found").into_response(),
    };

    let serve = tower_http::services::ServeFile::new(&file_path);
    match serve.oneshot(req).await {
        Ok(res) => {
            let mut response = res.into_response();
            let disp = format!("attachment; filename=\"{}\"", filename);
            if let Ok(hv) = header::HeaderValue::from_str(&disp) {
                response.headers_mut().insert(header::CONTENT_DISPOSITION, hv);
            }
            response
        }
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("File error: {}", err),
        )
            .into_response(),
    }
}

async fn handle_health() -> &'static str {
    "OK"
}
