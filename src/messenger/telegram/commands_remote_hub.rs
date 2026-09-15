//! # Remote Hub Services Detector and Runner
//!
//! Handles background process inspection, screen-based session spawning,
//! and network status checks for Antigravity Remote, Tailscale, and SSH.
//!
//! ## Search Tags
//! #remote-detector, #screen-spawn, #network-inspection

use std::net::TcpStream;
use std::process::Command;
use std::time::Duration;

#[derive(Clone, Debug, PartialEq)]
pub enum ServiceStatus {
    Active,
    Inactive,
    NotInstalled,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RemoteHubStatus {
    pub antigravity: ServiceStatus,
    pub antigravity_url: Option<String>,
    pub antigravity_uptime: Option<String>,
    pub antigravity_pid: Option<u32>,
    pub tailscale: ServiceStatus,
    pub tailscale_ip: Option<String>,
    pub ssh: ServiceStatus,
}

pub fn binary_exists(name: &str) -> bool {
    if let Ok(out) = Command::new("which").arg(name).output() {
        if out.status.success() && !out.stdout.is_empty() {
            return true;
        }
    }
    let home = std::env::var("HOME").unwrap_or_default();
    if !home.is_empty() {
        if std::path::Path::new(&format!("{}/.local/bin/{}", home, name)).exists() {
            return true;
        }
        if std::path::Path::new(&format!("{}/.gemini/antigravity-cli/bin/{}", home, name)).exists() {
            return true;
        }
    }
    false
}

pub fn find_antigravity_process() -> Option<(u32, String)> {
    let out = Command::new("pgrep").args(["-f", "agy --remote-control"]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let pids = String::from_utf8_lossy(&out.stdout);
    for line in pids.lines() {
        if let Ok(pid) = line.trim().parse::<u32>() {
            let cmd_path = format!("/proc/{}/cmdline", pid);
            if let Ok(cmd_bytes) = std::fs::read(&cmd_path) {
                let cmd_str = String::from_utf8_lossy(&cmd_bytes);
                if cmd_str.contains("--remote-control") && !cmd_str.contains("SCREEN") {
                    let mut uptime = "Active".to_string();
                    if let Ok(meta) = std::fs::metadata(format!("/proc/{}", pid)) {
                        if let Ok(mtime) = meta.created().or_else(|_| meta.modified()) {
                            if let Ok(elapsed) = std::time::SystemTime::now().duration_since(mtime) {
                                let total_mins = elapsed.as_secs() / 60;
                                let hours = total_mins / 60;
                                let mins = total_mins % 60;
                                uptime = if hours > 0 {
                                    format!("{}h {}m", hours, mins)
                                } else {
                                    format!("{}m", mins)
                                };
                            }
                        }
                    }
                    return Some((pid, uptime));
                }
            }
        }
    }
    None
}

pub fn extract_antigravity_url() -> Option<String> {
    let cache_file = "/tmp/tuner_agy_url.txt";
    if let Ok(cached) = std::fs::read_to_string(cache_file) {
        let trimmed = cached.trim();
        if trimmed.starts_with("https://antigravity.google.com/r/") {
            let normalized = if trimmed.ends_with("-v1") {
                format!("{}-v2", &trimmed[..trimmed.len() - 3])
            } else {
                trimmed.to_string()
            };
            return Some(normalized);
        }
    }

    for name in ["tuner-agy-remote", "agy-tuner"] {
        let tmp_screen = format!("/tmp/screen_{}.txt", name);
        let _ = Command::new("screen").args(["-S", name, "-X", "hardcopy", &tmp_screen]).output();
        if let Ok(bytes) = std::fs::read(&tmp_screen) {
            let text = String::from_utf8_lossy(&bytes);
            if let Some(pos) = text.find("https://antigravity.google.com/r/") {
                let raw_url: String = text[pos..].chars().take_while(|c| !c.is_whitespace() && *c != '\0').collect();
                if !raw_url.is_empty() {
                    let url = if raw_url.ends_with("-v1") {
                        format!("{}-v2", &raw_url[..raw_url.len() - 3])
                    } else {
                        raw_url
                    };
                    let _ = std::fs::write(cache_file, &url);
                    return Some(url);
                }
            }
        }
    }
    None
}

pub fn check_antigravity_status() -> (ServiceStatus, Option<String>, Option<String>, Option<u32>) {
    if !binary_exists("agy") {
        return (ServiceStatus::NotInstalled, None, None, None);
    }
    if let Some((pid, uptime)) = find_antigravity_process() {
        (ServiceStatus::Active, extract_antigravity_url(), Some(uptime), Some(pid))
    } else {
        (ServiceStatus::Inactive, None, None, None)
    }
}

pub fn check_tailscale_status() -> (ServiceStatus, Option<String>) {
    if !binary_exists("tailscale") {
        return (ServiceStatus::NotInstalled, None);
    }
    if let Ok(out) = Command::new("tailscale").args(["ip", "-4"]).output() {
        if out.status.success() {
            let ip = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !ip.is_empty() && ip.starts_with("100.") {
                return (ServiceStatus::Active, Some(ip));
            }
        }
    }
    (ServiceStatus::Inactive, None)
}

pub fn check_ssh_status() -> ServiceStatus {
    if let Ok(addr) = "127.0.0.1:22".parse() {
        if TcpStream::connect_timeout(&addr, Duration::from_millis(350)).is_ok() {
            return ServiceStatus::Active;
        }
    }
    if binary_exists("sshd") || binary_exists("ssh") {
        ServiceStatus::Inactive
    } else {
        ServiceStatus::NotInstalled
    }
}

pub fn query_remote_hub_status() -> RemoteHubStatus {
    let (agy_status, agy_url, agy_uptime, agy_pid) = check_antigravity_status();
    let (ts_status, ts_ip) = check_tailscale_status();
    RemoteHubStatus {
        antigravity: agy_status,
        antigravity_url: agy_url,
        antigravity_uptime: agy_uptime,
        antigravity_pid: agy_pid,
        tailscale: ts_status,
        tailscale_ip: ts_ip,
        ssh: check_ssh_status(),
    }
}

pub async fn start_antigravity_remote() -> Result<String, String> {
    let _ = stop_antigravity_remote().await;
    let status = Command::new("screen")
        .args(["-dmS", "tuner-agy-remote", "bash", "-c", "agy --remote-control --remote-control-name wimvm-dev"])
        .status()
        .map_err(|e| format!("Spawn error: {}", e))?;

    if !status.success() {
        return Err("Screen spawn failed".to_string());
    }

    for _ in 0..8 {
        tokio::time::sleep(Duration::from_millis(500)).await;
        if let Some(url) = extract_antigravity_url() {
            return Ok(url);
        }
    }
    Ok("https://antigravity.google.com".to_string())
}

pub async fn stop_antigravity_remote() -> Result<(), String> {
    let _ = Command::new("screen").args(["-S", "tuner-agy-remote", "-X", "quit"]).output();
    let _ = Command::new("screen").args(["-S", "agy-tuner", "-X", "quit"]).output();
    let _ = Command::new("pkill").args(["-f", "agy --remote-control"]).output();
    let _ = std::fs::remove_file("/tmp/tuner_agy_url.txt");
    let _ = std::fs::remove_file("/tmp/screen_tuner-agy-remote.txt");
    let _ = std::fs::remove_file("/tmp/screen_agy-tuner.txt");
    tokio::time::sleep(Duration::from_millis(300)).await;
    Ok(())
}
