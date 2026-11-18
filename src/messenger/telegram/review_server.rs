//! # In-App File Review Web Server
//!
//! ## Overview
//! Provides the Axum HTTP web server and Cloudflare Quick Tunnel orchestration
//! for file review, code inspection, media streaming (HTTP Range/206), and downloads.
//!
//! ## Search Tags
//! #review-server, #cloudflare-tunnel, #media-streaming

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
use tokio::io::AsyncBufReadExt;
use tokio::sync::Mutex;
use tower::ServiceExt;

/// Executes `ensure_server_running` (ensure server running).
pub async fn ensure_server_running(server_port: &Arc<Mutex<Option<u16>>>) -> u16 {
    let mut port_guard = server_port.lock().await;
    if let Some(port) = *port_guard {
        if tokio::net::TcpStream::connect(("127.0.0.1", port)).await.is_ok() {
            return port;
        }
        *port_guard = None;
    }

    let bind_addr = if cfg!(test) {
        "0.0.0.0:0".to_string()
    } else if let Ok(raw_p) = std::env::var("TUNER_WEB_PORT")
        && let Ok(p) = raw_p.trim().parse::<u16>()
        && p > 0
    {
        format!("0.0.0.0:{p}")
    } else {
        "0.0.0.0:8743".to_string()
    };

    let listener = match tokio::net::TcpListener::bind(&bind_addr).await {
        Ok(l) => l,
        Err(_) => match tokio::net::TcpListener::bind("0.0.0.0:0").await {
            Ok(l) => l,
            Err(_) => return 8743,
        },
    };

    let port = listener.local_addr().map(|a| a.port()).unwrap_or(8743);
    *port_guard = Some(port);

    let app = build_review_router();

    tokio::spawn(async move {
        let svc = app.into_make_service_with_connect_info::<std::net::SocketAddr>();
        axum::serve(listener, svc).await.ok();
    });

    port
}

/// Builds the combined Axum router for `/review` and `/vault` endpoints.
pub fn build_review_router() -> Router {
    Router::new()
        .route("/review/:token", get(handle_review_page))
        .route("/review/:token/json", get(handle_review_json))
        .route("/review/:token/media/:filename", get(handle_media))
        .route("/review/:token/download/:filename", get(handle_download))
        .route("/vault/:token", get(super::vault::handle_vault_page))
        .route(
            "/vault/:token/api/entries",
            get(super::vault::handle_vault_list).post(super::vault::handle_vault_upsert),
        )
        .route(
            "/vault/:token/api/entries/:site_key",
            axum::routing::delete(super::vault::handle_vault_delete),
        )
        .route(
            "/vault/:token/api/lock",
            axum::routing::post(super::vault::handle_vault_lock),
        )
        .merge(crate::security::vault_auth::proxy::auth_proxy_routes())
        .merge(crate::messenger::web::dashboard_routes())
        .route("/health", get(handle_health))
}

/// Executes `warmup` (warmup).
pub async fn warmup(server_port: &Arc<Mutex<Option<u16>>>, tunnel_url: &Arc<Mutex<Option<String>>>) {
    let port = ensure_server_running(server_port).await;
    let mut tunnel_guard = tunnel_url.lock().await;
    if tunnel_guard.is_none()
        && let Some(url) = spawn_quick_tunnel_tracked(port, Some(tunnel_url.clone())).await {
            *tunnel_guard = Some(url);
        }
}

/// Executes `get_review_url` (get review url).
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

    if let Some(url) = spawn_quick_tunnel_tracked(port, Some(tunnel_url.clone())).await {
        *tunnel_guard = Some(url.clone());
        format!("{}/review/{}", url, token)
    } else {
        format!("http://127.0.0.1:{}/review/{}", port, token)
    }
}

/// Resolves the public or loopback URL for `/vault/:token`.
pub async fn get_vault_url(
    server_port: &Arc<Mutex<Option<u16>>>,
    tunnel_url: &Arc<Mutex<Option<String>>>,
    token: &str,
) -> String {
    resolve_tunneled_path(server_port, tunnel_url, "vault", token).await
}

/// Resolves the public or loopback URL for `/auth/:token`.
pub async fn get_auth_url(
    server_port: &Arc<Mutex<Option<u16>>>,
    tunnel_url: &Arc<Mutex<Option<String>>>,
    token: &str,
) -> String {
    resolve_tunneled_path(server_port, tunnel_url, "auth", token).await
}

async fn resolve_tunneled_path(
    server_port: &Arc<Mutex<Option<u16>>>,
    tunnel_url: &Arc<Mutex<Option<String>>>,
    prefix: &str,
    token: &str,
) -> String {
    let port = ensure_server_running(server_port).await;
    let mut tunnel_guard = tunnel_url.lock().await;
    if let Some(ref base) = *tunnel_guard {
        return format!("{}/{}/{}", base, prefix, token);
    }

    if let Some(url) = spawn_quick_tunnel_tracked(port, Some(tunnel_url.clone())).await {
        *tunnel_guard = Some(url.clone());
        format!("{}/{}/{}", url, prefix, token)
    } else {
        format!("http://127.0.0.1:{}/{}/{}", port, prefix, token)
    }
}

fn find_cloudflared_bin() -> Option<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    crate::setup::doctor::find_cloudflared_bin(&home)
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

fn spawn_tunnel_monitor(
    mut child: tokio::process::Child,
    mut reader: tokio::io::BufReader<tokio::process::ChildStderr>,
    url: String,
    slot: Option<Arc<Mutex<Option<String>>>>,
) {
    crate::messenger::web::cli_dash::publish_global_tunnel_url(Some(&url));
    tokio::spawn(async move {
        let mut discard = String::new();
        while reader.read_line(&mut discard).await.unwrap_or(0) > 0 {
            discard.clear();
        }
        let _ = child.wait().await;
        crate::messenger::web::cli_dash::publish_global_tunnel_url(None);
        if let Some(s) = slot {
            let mut g = s.lock().await;
            if g.as_deref() == Some(url.as_str()) {
                *g = None;
            }
        }
    });
}

async fn spawn_quick_tunnel_tracked(
    port: u16,
    tunnel_url_slot: Option<Arc<Mutex<Option<String>>>>,
) -> Option<String> {
    let bin = find_cloudflared_bin()?;
    let mut cmd = tokio::process::Command::new(bin);
    cmd.args(["tunnel", "--url", &format!("http://127.0.0.1:{}", port)])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    #[cfg(target_os = "linux")]
    unsafe {
        cmd.pre_exec(|| {
            nix::libc::prctl(nix::libc::PR_SET_PDEATHSIG, nix::libc::SIGTERM);
            Ok(())
        });
    }
    let mut child = cmd.spawn().ok()?;
    let mut reader = tokio::io::BufReader::new(child.stderr.take()?);
    let mut line = String::new();
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(10) {
        line.clear();
        let rem = Duration::from_secs(10).saturating_sub(start.elapsed());
        if tokio::time::timeout(rem, reader.read_line(&mut line)).await.ok().and_then(Result::ok).unwrap_or(0) == 0 {
            break;
        }
        if let Some(url) = parse_tunnel_url(&line) {
            spawn_tunnel_monitor(child, reader, url.clone(), tunnel_url_slot);
            return Some(url);
        }
    }
    let _ = child.kill().await;
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
