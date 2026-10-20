//! # Remote Hub Services Detector and Runner
//!
//! ## Overview
//! Handles background process inspection, screen spawning, and network status.
//!
//! ## Search Tags
//! #remote-detector, #screen-spawn, #network-inspection

use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

#[derive(Clone, Debug, PartialEq)]
/// Enumerates `ServiceStatus` (service status) variants.
pub enum ServiceStatus {
    /// `Active` (active) variant.
    Active,
    /// `Inactive` (inactive) variant.
    Inactive,
    /// `NotInstalled` (not installed) variant.
    NotInstalled,
}

#[derive(Clone, Debug, PartialEq)]
/// Represents `RemoteHubStatus` (remote hub status).
pub struct RemoteHubStatus {
    /// Profile.
    pub profile: String,
    /// Remote workspace.
    pub remote_workspace: PathBuf,
    /// Antigravity.
    pub antigravity: ServiceStatus,
    /// Antigravity url.
    pub antigravity_url: Option<String>,
    /// Antigravity uptime.
    pub antigravity_uptime: Option<String>,
    /// Antigravity pid.
    pub antigravity_pid: Option<u32>,
    /// Tailscale.
    pub tailscale: ServiceStatus,
    /// Tailscale ip.
    pub tailscale_ip: Option<String>,
    /// Ssh.
    pub ssh: ServiceStatus,
}

/// Executes `resolve_profile_info` (resolve profile info).
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

/// Executes `binary_exists` (binary exists).
pub fn binary_exists(name: &str) -> bool {
    if let Ok(out) = Command::new("which").arg(name).output()
        && out.status.success() && !out.stdout.is_empty() {
            return true;
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

/// Executes `find_antigravity_process` (find antigravity process).
pub fn find_antigravity_process(instance_name: &str) -> Option<(u32, String)> {
    let out = Command::new("pgrep").args(["-f", "agy --remote-control"]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let mut fallback = None;
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        if let Ok(pid) = line.trim().parse::<u32>() {
            let cmd_path = format!("/proc/{}/cmdline", pid);
            if let Ok(bytes) = std::fs::read(&cmd_path) {
                let s = String::from_utf8_lossy(&bytes);
                let is_wrapper = s.contains("SCREEN") || s.starts_with("script\0") || s.starts_with("bash\0") || s.starts_with("sh\0");
                if s.contains("--remote-control") && !is_wrapper {
                    let is_match = s.contains(instance_name)
                        || (instance_name.starts_with("default") && (s.contains("wimvm-dev") || !s.contains("-remote")));
                    if is_match {
                        let mut uptime = "Active".to_string();
                        if let Ok(meta) = std::fs::metadata(format!("/proc/{}", pid))
                            && let Ok(mtime) = meta.created().or_else(|_| meta.modified())
                                && let Ok(el) = std::time::SystemTime::now().duration_since(mtime) {
                                    let m = el.as_secs() / 60;
                                    uptime = if m >= 60 { format!("{}h {}m", m / 60, m % 60) } else { format!("{}m", m) };
                                }
                        if s.starts_with("agy\0") || s.contains("/agy\0") {
                            return Some((pid, uptime));
                        }
                        fallback = Some((pid, uptime));
                    }
                }
            }
        }
    }
    fallback
}

/// Parses and normalizes an Antigravity remote control URL from raw terminal output.
pub fn parse_antigravity_url_from_text(text: &str) -> Option<String> {
    let prefix = "https://antigravity.google.com/r/";
    let pos = text.find(prefix)?;
    let slug: String = text[pos + prefix.len()..]
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    if slug.is_empty() {
        return None;
    }
    let raw = format!("{}{}", prefix, slug);
    let url = if let Some(base) = raw.strip_suffix("-v1") {
        format!("{}-v2", base)
    } else {
        raw
    };
    Some(url)
}

/// Executes `extract_antigravity_url` (extract antigravity url).
pub fn extract_antigravity_url(profile: &str, screen_name: &str) -> Option<String> {
    let cache_file = format!("/tmp/tuner_agy_url_{}.txt", profile);
    if let Ok(c) = std::fs::read_to_string(&cache_file)
        && let Some(url) = parse_antigravity_url_from_text(c.trim()) {
            return Some(url);
        }

    let tmp_screen = format!("/tmp/screen_{}.txt", screen_name);
    if binary_exists("screen") {
        let _ = Command::new("screen").args(["-S", screen_name, "-X", "hardcopy", &tmp_screen]).output();
    }
    if let Ok(bytes) = std::fs::read(&tmp_screen) {
        let text = String::from_utf8_lossy(&bytes);
        if let Some(url) = parse_antigravity_url_from_text(&text) {
            let _ = std::fs::write(&cache_file, &url);
            return Some(url);
        }
    }
    None
}

/// Executes `check_antigravity_status` (check antigravity status).
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

/// Executes `check_tailscale_status` (check tailscale status).
pub fn check_tailscale_status() -> (ServiceStatus, Option<String>) {
    if !binary_exists("tailscale") {
        return (ServiceStatus::NotInstalled, None);
    }
    if let Ok(out) = Command::new("tailscale").args(["ip", "-4"]).output()
        && out.status.success() {
            let ip = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !ip.is_empty() && ip.starts_with("100.") {
                return (ServiceStatus::Active, Some(ip));
            }
        }
    (ServiceStatus::Inactive, None)
}

/// Executes `check_ssh_status` (check ssh status).
pub fn check_ssh_status() -> ServiceStatus {
    if let Ok(addr) = "127.0.0.1:22".parse()
        && TcpStream::connect_timeout(&addr, Duration::from_millis(350)).is_ok() {
            return ServiceStatus::Active;
        }
    if binary_exists("sshd") || binary_exists("ssh") {
        ServiceStatus::Inactive
    } else {
        ServiceStatus::NotInstalled
    }
}

/// Executes `query_remote_hub_status` (query remote hub status).
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

/// Executes `start_antigravity_remote` (start antigravity remote).
pub async fn start_antigravity_remote(working_dir: &Path) -> Result<String, String> {
    let (profile, remote_ws, screen_name, instance_name) = resolve_profile_info(working_dir);
    let _ = stop_antigravity_remote(working_dir).await;
    let _ = std::fs::create_dir_all(&remote_ws);
    crate::cli::antigravity::trust::trust_workspace_in_settings(&remote_ws, None);

    let cmd = format!(
        "SSH_CONNECTION='127.0.0.1 0 127.0.0.1 22' agy --remote-control --remote-control-name {} --add-dir {}",
        instance_name,
        remote_ws.display()
    );
    if binary_exists("screen") {
        let status = Command::new("screen")
            .args(["-dmS", &screen_name, "bash", "-c", &cmd])
            .current_dir(&remote_ws)
            .status()
            .map_err(|e| format!("Spawn error: {}", e))?;
        if !status.success() {
            return Err("Screen spawn failed".to_string());
        }
    } else {
        let tmp_screen = format!("/tmp/screen_{}.txt", screen_name);
        Command::new("setsid")
            .args(["script", "-q", "-f", "-c", &cmd, &tmp_screen])
            .current_dir(&remote_ws)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| format!("Spawn error: {}", e))?;
    }

    for _ in 0..10 {
        tokio::time::sleep(Duration::from_millis(500)).await;
        if let Some(url) = extract_antigravity_url(&profile, &screen_name) {
            return Ok(url);
        }
    }
    Ok("https://antigravity.google.com".to_string())
}

/// Executes `stop_antigravity_remote` (stop antigravity remote).
pub async fn stop_antigravity_remote(working_dir: &Path) -> Result<(), String> {
    let (profile, _remote_ws, screen_name, instance_name) = resolve_profile_info(working_dir);
    if binary_exists("screen") {
        let _ = Command::new("screen").args(["-S", &screen_name, "-X", "quit"]).output();
        if profile == "default" {
            let _ = Command::new("screen").args(["-S", "tuner-agy-remote", "-X", "quit"]).output();
            let _ = Command::new("screen").args(["-S", "agy-tuner", "-X", "quit"]).output();
        }
    }
    if profile == "default" {
        let _ = Command::new("pkill").args(["-15", "-f", "--", "wimvm-dev"]).output();
    }
    let pattern = format!("--remote-control-name {}", instance_name);
    let _ = Command::new("pkill").args(["-15", "-f", "--", &pattern]).output();
    let _ = std::fs::remove_file(format!("/tmp/tuner_agy_url_{}.txt", profile));
    let _ = std::fs::remove_file(format!("/tmp/screen_{}.txt", screen_name));

    for _ in 0..10 {
        tokio::time::sleep(Duration::from_millis(200)).await;
        if find_antigravity_process(&instance_name).is_none() {
            return Ok(());
        }
    }

    let _ = Command::new("pkill").args(["-9", "-f", "--", &pattern]).output();
    tokio::time::sleep(Duration::from_millis(150)).await;
    Ok(())
}
