//! # Remote Hub Services Detector and Runner
//!
//! Handles background process inspection, screen spawning, and network status.
//! #remote-detector, #screen-spawn, #network-inspection

use std::net::TcpStream;
use std::path::{Path, PathBuf};
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
    pub profile: String,
    pub remote_workspace: PathBuf,
    pub antigravity: ServiceStatus,
    pub antigravity_url: Option<String>,
    pub antigravity_uptime: Option<String>,
    pub antigravity_pid: Option<u32>,
    pub tailscale: ServiceStatus,
    pub tailscale_ip: Option<String>,
    pub ssh: ServiceStatus,
}

pub fn resolve_profile_info(working_dir: &Path) -> (String, PathBuf, String, String) {
    let profile = working_dir
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .filter(|n| *n != "profiles" && !n.is_empty())
        .unwrap_or("default")
        .to_string();
    let remote_ws = working_dir
        .parent()
        .map(|p| p.join("remote_workspace"))
        .unwrap_or_else(|| working_dir.join("remote_workspace"));
    let screen_name = format!("tuner-agy-remote-{}", profile);
    let instance_name = format!("{}-remote", profile);
    (profile, remote_ws, screen_name, instance_name)
}

pub fn binary_exists(name: &str) -> bool {
    if let Ok(out) = Command::new("which").arg(name).output() {
        if out.status.success() && !out.stdout.is_empty() {
            return true;
        }
    }
    let home = std::env::var("HOME").unwrap_or_default();
    if !home.is_empty() {
        for p in [".local/bin", ".gemini/antigravity-cli/bin"] {
            if Path::new(&format!("{}/{}/{}", home, p, name)).exists() {
                return true;
            }
        }
    }
    false
}

pub fn find_antigravity_process(instance_name: &str) -> Option<(u32, String)> {
    let out = Command::new("pgrep").args(["-f", "agy --remote-control"]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        if let Ok(pid) = line.trim().parse::<u32>() {
            let cmd_path = format!("/proc/{}/cmdline", pid);
            if let Ok(bytes) = std::fs::read(&cmd_path) {
                let s = String::from_utf8_lossy(&bytes);
                if s.contains("--remote-control") && !s.contains("SCREEN") {
                    let is_match = s.contains(instance_name)
                        || (instance_name.starts_with("default") && (s.contains("wimvm-dev") || !s.contains("-remote")));
                    if is_match {
                        let mut uptime = "Active".to_string();
                        if let Ok(meta) = std::fs::metadata(format!("/proc/{}", pid)) {
                            if let Ok(mtime) = meta.created().or_else(|_| meta.modified()) {
                                if let Ok(el) = std::time::SystemTime::now().duration_since(mtime) {
                                    let m = el.as_secs() / 60;
                                    uptime = if m >= 60 { format!("{}h {}m", m / 60, m % 60) } else { format!("{}m", m) };
                                }
                            }
                        }
                        return Some((pid, uptime));
                    }
                }
            }
        }
    }
    None
}

pub fn extract_antigravity_url(profile: &str, screen_name: &str) -> Option<String> {
    let cache_file = format!("/tmp/tuner_agy_url_{}.txt", profile);
    if let Ok(c) = std::fs::read_to_string(&cache_file) {
        let t = c.trim();
        if t.starts_with("https://antigravity.google.com/r/") {
            let norm = if t.ends_with("-v1") { format!("{}-v2", &t[..t.len() - 3]) } else { t.to_string() };
            return Some(norm);
        }
    }

    let tmp_screen = format!("/tmp/screen_{}.txt", screen_name);
    let _ = Command::new("screen").args(["-S", screen_name, "-X", "hardcopy", &tmp_screen]).output();
    if let Ok(bytes) = std::fs::read(&tmp_screen) {
        let text = String::from_utf8_lossy(&bytes);
        if let Some(pos) = text.find("https://antigravity.google.com/r/") {
            let raw: String = text[pos..].chars().take_while(|c| !c.is_whitespace() && *c != '\0').collect();
            if !raw.is_empty() {
                let url = if raw.ends_with("-v1") { format!("{}-v2", &raw[..raw.len() - 3]) } else { raw };
                let _ = std::fs::write(&cache_file, &url);
                return Some(url);
            }
        }
    }
    None
}

pub fn check_antigravity_status(instance_name: &str, profile: &str, screen_name: &str) -> (ServiceStatus, Option<String>, Option<String>, Option<u32>) {
    if !binary_exists("agy") {
        return (ServiceStatus::NotInstalled, None, None, None);
    }
    if let Some((pid, uptime)) = find_antigravity_process(instance_name) {
        (ServiceStatus::Active, extract_antigravity_url(profile, screen_name), Some(uptime), Some(pid))
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

pub fn query_remote_hub_status(working_dir: &Path) -> RemoteHubStatus {
    let (profile, remote_ws, screen_name, instance_name) = resolve_profile_info(working_dir);
    let (agy_status, agy_url, agy_uptime, agy_pid) = check_antigravity_status(&instance_name, &profile, &screen_name);
    let (ts_status, ts_ip) = check_tailscale_status();
    RemoteHubStatus {
        profile,
        remote_workspace: remote_ws,
        antigravity: agy_status,
        antigravity_url: agy_url,
        antigravity_uptime: agy_uptime,
        antigravity_pid: agy_pid,
        tailscale: ts_status,
        tailscale_ip: ts_ip,
        ssh: check_ssh_status(),
    }
}

pub async fn start_antigravity_remote(working_dir: &Path) -> Result<String, String> {
    let (profile, remote_ws, screen_name, instance_name) = resolve_profile_info(working_dir);
    let _ = stop_antigravity_remote(working_dir).await;
    let _ = std::fs::create_dir_all(&remote_ws);

    let cmd = format!("agy --remote-control --remote-control-name {} --add-dir {}", instance_name, remote_ws.display());
    let status = Command::new("screen")
        .args(["-dmS", &screen_name, "bash", "-c", &cmd])
        .current_dir(&remote_ws)
        .status()
        .map_err(|e| format!("Spawn error: {}", e))?;

    if !status.success() {
        return Err("Screen spawn failed".to_string());
    }

    for _ in 0..8 {
        tokio::time::sleep(Duration::from_millis(500)).await;
        if let Some(url) = extract_antigravity_url(&profile, &screen_name) {
            return Ok(url);
        }
    }
    Ok("https://antigravity.google.com".to_string())
}

pub async fn stop_antigravity_remote(working_dir: &Path) -> Result<(), String> {
    let (profile, _remote_ws, screen_name, instance_name) = resolve_profile_info(working_dir);
    let _ = Command::new("screen").args(["-S", &screen_name, "-X", "quit"]).output();
    if profile == "default" {
        let _ = Command::new("screen").args(["-S", "tuner-agy-remote", "-X", "quit"]).output();
        let _ = Command::new("screen").args(["-S", "agy-tuner", "-X", "quit"]).output();
        let _ = Command::new("pkill").args(["-15", "-f", "wimvm-dev"]).output();
    }
    let _ = Command::new("pkill").args(["-15", "-f", &format!("--remote-control-name {}", instance_name)]).output();
    let _ = std::fs::remove_file(format!("/tmp/tuner_agy_url_{}.txt", profile));
    let _ = std::fs::remove_file(format!("/tmp/screen_{}.txt", screen_name));

    for _ in 0..10 {
        tokio::time::sleep(Duration::from_millis(200)).await;
        if find_antigravity_process(&instance_name).is_none() {
            return Ok(());
        }
    }

    let _ = Command::new("pkill").args(["-9", "-f", &format!("--remote-control-name {}", instance_name)]).output();
    tokio::time::sleep(Duration::from_millis(150)).await;
    Ok(())
}
