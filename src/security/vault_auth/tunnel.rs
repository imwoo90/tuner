//! # Warm `:8743` Proxy Registration & Standalone Quick Tunnel Fallback (`tunnel.rs`)
//!
//! ## Overview
//! Exposes the local `127.0.0.1` HITL auth server externally by either registering on the
//! running worker's warm `:8743` Cloudflare Quick Tunnel (`0ms` reuse) or spawning a standalone
//! `cloudflared` tunnel, polling edge `HTTP 200` readiness before emitting the URL.
//!
//! ## Search Tags
//! #quick-tunnel, #edge-readiness, #warm-tunnel-fallback, #cloudflared

use super::proxy::{
    load_tunnel_endpoint, RegisterAuthRouteReq, TunnelEndpointInfo, UnregisterAuthRouteReq,
};
use crate::cli::common::kill_process_group;
use crate::security::vault::current_unix_secs;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

const IPC_HEADER: &str = "x-tuner-ipc-secret";

/// Child process handles for a standalone `cloudflared` tunnel.
pub struct StandaloneTunnelHandle {
    /// `cloudflared` child handle.
    pub child: Child,
    /// `cloudflared` OS PID (process group leader).
    pub pid: Option<u32>,
    /// Parent-death reaper child.
    pub watchdog: Option<Child>,
}

/// Active external exposure handle for a running HITL auth session.
pub enum ActiveTunnel {
    /// Registered route on the running worker's `:8743` server.
    WarmProxy {
        /// Worker loopback IPC info.
        endpoint: TunnelEndpointInfo,
        /// Registered one-time token.
        token: String,
    },
    /// Dedicated standalone `cloudflared` child process.
    Standalone(Box<StandaloneTunnelHandle>),
    /// Local loopback URL only (`--no-tunnel`).
    LoopbackOnly,
}

/// Resolves or spawns the public URL for `127.0.0.1:<local_port>/auth/<token>`.
pub async fn establish_auth_tunnel(
    vault_dir: &Path,
    local_port: u16,
    token: &str,
    ttl_secs: u64,
    no_tunnel: bool,
) -> Result<(String, ActiveTunnel), String> {
    let loopback_url = format!("http://127.0.0.1:{local_port}/auth/{token}");
    if no_tunnel {
        return Ok((loopback_url, ActiveTunnel::LoopbackOnly));
    }

    if let Some(ep) = load_tunnel_endpoint(vault_dir)
        && let Some(pub_url) = try_register_warm_proxy(&ep, local_port, token, ttl_secs).await
    {
        if pub_url.starts_with("https://") {
            wait_for_http_ready(&pub_url, Duration::from_secs(12)).await;
        }
        return Ok((
            pub_url,
            ActiveTunnel::WarmProxy {
                endpoint: ep,
                token: token.to_string(),
            },
        ));
    }

    spawn_standalone_tunnel(local_port, token).await
}

async fn try_register_warm_proxy(
    ep: &TunnelEndpointInfo,
    local_port: u16,
    token: &str,
    ttl_secs: u64,
) -> Option<String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(12))
        .build()
        .ok()?;
    let req = RegisterAuthRouteReq {
        token: token.to_string(),
        local_port,
        expires_at_secs: current_unix_secs().saturating_add(ttl_secs),
    };
    let resp = client
        .post(format!(
            "http://127.0.0.1:{}/internal/auth/register",
            ep.ipc_port
        ))
        .header(IPC_HEADER, &ep.ipc_secret)
        .json(&req)
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let body: Value = resp.json().await.ok()?;
    let url = body.get("public_url")?.as_str()?.trim().to_string();
    if url.starts_with("https://") {
        Some(url)
    } else {
        let _ = unregister_warm_proxy(ep, token).await;
        None
    }
}

async fn unregister_warm_proxy(ep: &TunnelEndpointInfo, token: &str) {
    let Ok(client) = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
    else {
        return;
    };
    let req = UnregisterAuthRouteReq {
        token: token.to_string(),
    };
    let _ = client
        .post(format!(
            "http://127.0.0.1:{}/internal/auth/unregister",
            ep.ipc_port
        ))
        .header(IPC_HEADER, &ep.ipc_secret)
        .json(&req)
        .send()
        .await;
}

fn find_cloudflared() -> Result<PathBuf, String> {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    crate::setup::doctor::find_cloudflared_bin(&home).ok_or_else(|| {
        "cloudflared binary not found. Run `tuner --doctor --fix` first.".to_string()
    })
}

fn extract_trycloudflare_base(line: &str) -> Option<String> {
    if line.contains("api.trycloudflare.com") {
        return None;
    }
    let pos = line.find("https://")?;
    let sub = &line[pos..];
    let end = sub.find(".trycloudflare.com")?;
    let candidate = sub[..end + ".trycloudflare.com".len()].trim();
    if candidate == "https://trycloudflare.com" || candidate.contains("api.trycloudflare.com") {
        None
    } else {
        Some(candidate.to_string())
    }
}

async fn read_cloudflared_base(stderr: tokio::process::ChildStderr) -> Option<String> {
    let mut reader = BufReader::new(stderr);
    let mut line = String::new();
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(15) {
        line.clear();
        let rem = Duration::from_secs(15).saturating_sub(start.elapsed());
        let Ok(Ok(n)) = tokio::time::timeout(rem, reader.read_line(&mut line)).await else {
            break;
        };
        if n == 0 {
            break;
        }
        if let Some(found) = extract_trycloudflare_base(&line) {
            tokio::spawn(async move {
                let mut discard = String::new();
                while reader.read_line(&mut discard).await.unwrap_or(0) > 0 {
                    discard.clear();
                }
            });
            return Some(found);
        }
    }
    None
}

async fn spawn_standalone_tunnel(
    local_port: u16,
    token: &str,
) -> Result<(String, ActiveTunnel), String> {
    let bin = find_cloudflared()?;
    let mut child = Command::new(bin)
        .args([
            "tunnel",
            "--no-autoupdate",
            "--url",
            &format!("http://127.0.0.1:{local_port}"),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .process_group(0)
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("Failed to spawn cloudflared: {e}"))?;
    let pid = child.id();
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "Failed to read cloudflared stderr".to_string())?;

    let Some(base) = read_cloudflared_base(stderr).await else {
        if let Some(p) = pid {
            kill_process_group(p);
        }
        let _ = child.kill().await;
        return Err("Timed out waiting for Cloudflare Quick Tunnel URL".to_string());
    };

    let watchdog = pid.and_then(spawn_cf_watchdog);
    let full_url = format!("{base}/auth/{token}");
    wait_for_http_ready(&full_url, Duration::from_secs(18)).await;

    Ok((
        full_url,
        ActiveTunnel::Standalone(Box::new(StandaloneTunnelHandle {
            child,
            pid,
            watchdog,
        })),
    ))
}

fn spawn_cf_watchdog(cf_pgid: u32) -> Option<Child> {
    let ppid = std::process::id();
    let script = format!(
        "while kill -0 {ppid} 2>/dev/null; do sleep 1; done; \
         kill -TERM -{cf_pgid} 2>/dev/null || true; sleep 1; \
         kill -KILL -{cf_pgid} 2>/dev/null || true"
    );
    Command::new("sh")
        .args(["-c", &script])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .ok()
}

async fn wait_for_http_ready(url: &str, max_wait: Duration) {
    let Ok(client) = reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
    else {
        return;
    };
    let start = Instant::now();
    while start.elapsed() < max_wait {
        if let Ok(resp) = client.get(url).send().await
            && resp.status().is_success()
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(450)).await;
    }
}

impl ActiveTunnel {
    /// Unregisters the `:8743` route or terminates the standalone `cloudflared` tunnel.
    pub async fn teardown(&mut self) {
        match self {
            Self::WarmProxy { endpoint, token } => {
                unregister_warm_proxy(endpoint, token).await;
            }
            Self::Standalone(handle) => {
                if let Some(mut wd) = handle.watchdog.take() {
                    let _ = wd.kill().await;
                    let _ = wd.wait().await;
                }
                if let Some(p) = handle.pid {
                    kill_process_group(p);
                }
                let _ = handle.child.kill().await;
                let _ = handle.child.wait().await;
            }
            Self::LoopbackOnly => {}
        }
    }
}
