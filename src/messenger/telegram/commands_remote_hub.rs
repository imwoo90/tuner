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
    let parent = working_dir.parent();
    let profile = parent.and_then(|p| p.file_name()).and_then(|n| n.to_str())
        .filter(|n| *n != "profiles" && !n.is_empty()).unwrap_or("default").to_string();
    let remote_ws = parent.unwrap_or(working_dir).join("remote_workspace");
    (profile.clone(), remote_ws, format!("tuner-agy-remote-{profile}"), format!("{profile}-remote"))
}

/// Executes `binary_exists` (binary exists).
pub fn binary_exists(name: &str) -> bool {
    if Command::new("which").arg(name).output().is_ok_and(|o| o.status.success() && !o.stdout.is_empty()) {
        return true;
    }
    let home = std::env::var("HOME").unwrap_or_default();
    !home.is_empty() && [".local/bin", ".gemini/antigravity-cli/bin"].iter().any(|p| Path::new(&format!("{home}/{p}/{name}")).exists())
}

/// Executes `find_antigravity_process` (find antigravity process).
pub fn find_antigravity_process(name: &str) -> Option<(u32, String)> {
    let out = Command::new("pgrep").args(["-f", "remote-control"]).output().ok()?;
    if !out.status.success() { return None; }
    let mut fb = None;
    for l in String::from_utf8_lossy(&out.stdout).lines() {
        let Ok(pid) = l.trim().parse::<u32>() else { continue };
        let Ok(b) = std::fs::read(format!("/proc/{pid}/cmdline")) else { continue };
        let s = String::from_utf8_lossy(&b);
        let wrap = s.contains("SCREEN") || ["script\0", "bash\0", "sh\0"].iter().any(|p| s.starts_with(p));
        if (s.contains("--remote-control") || s.contains("remote-control\0serve")) && !wrap
            && (s.contains(name) || (name.starts_with("default") && (s.contains("wimvm-dev") || !s.contains("-remote"))))
        {
            let up = std::fs::metadata(format!("/proc/{pid}")).ok()
                .and_then(|m| m.created().or_else(|_| m.modified()).ok())
                .and_then(|t| std::time::SystemTime::now().duration_since(t).ok())
                .map(|el| { let m = el.as_secs() / 60; if m >= 60 { format!("{}h {}m", m / 60, m % 60) } else { format!("{m}m") } })
                .unwrap_or_else(|| "Active".to_string());
            if s.starts_with("agy\0") || s.contains("/agy\0") { return Some((pid, up)); }
            fb = Some((pid, up));
        }
    }
    fb
}

/// Parses `installation_uuid` from `antigravity_state.pbtxt` into a permanent `-v2` remote URL.
pub fn parse_installation_uuid_url(pbtxt: &str) -> Option<String> {
    pbtxt.lines().find_map(|l| {
        let u = l.trim().strip_prefix("installation_uuid:")?.trim().trim_matches('"').trim();
        (!u.is_empty() && u.chars().all(|c| c.is_ascii_hexdigit() || c == '-')).then(|| format!("https://antigravity.google.com/r/{u}-v2"))
    })
}

fn ensure_remote_control_hostname(name: &str) {
    let Ok(home) = std::env::var("HOME") else { return };
    let p = Path::new(&home).join(".gemini/config/config.json");
    if let Some(d) = p.parent() { let _ = std::fs::create_dir_all(d); }
    let mut v = std::fs::read_to_string(&p).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_else(|| serde_json::json!({}));
    if let Some(o) = v.as_object_mut() {
        o.insert("cliRemoteControlHostname".into(), name.into());
        if let Ok(s) = serde_json::to_string_pretty(&v) { let _ = std::fs::write(&p, s); }
    }
}

/// Parses and normalizes an Antigravity remote control URL from raw terminal output.
pub fn parse_antigravity_url_from_text(text: &str) -> Option<String> {
    let pfx = "https://antigravity.google.com/r/";
    let slug: String = text[text.find(pfx)? + pfx.len()..].chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
    (!slug.is_empty()).then(|| { let r = format!("{pfx}{slug}"); r.strip_suffix("-v1").map(|b| format!("{b}-v2")).unwrap_or(r) })
}

/// Executes `extract_antigravity_url` (extract antigravity url).
pub fn extract_antigravity_url(profile: &str, screen: &str) -> Option<String> {
    let cache = format!("/tmp/tuner_agy_url_{profile}.txt");
    if let Ok(c) = std::fs::read_to_string(&cache) && let Some(u) = parse_antigravity_url_from_text(c.trim()) { return Some(u); }
    let tmp = format!("/tmp/screen_{screen}.txt");
    if binary_exists("screen") { let _ = Command::new("screen").args(["-S", screen, "-X", "hardcopy", &tmp]).output(); }
    if let Ok(b) = std::fs::read(&tmp) && let Some(u) = parse_antigravity_url_from_text(&String::from_utf8_lossy(&b)) {
        let _ = std::fs::write(&cache, &u);
        return Some(u);
    }
    let home = std::env::var("HOME").ok()?;
    ["antigravity_state.pbtxt", "jetski_state.pbtxt"].iter().find_map(|f| {
        let u = parse_installation_uuid_url(&std::fs::read_to_string(Path::new(&home).join(".gemini/antigravity-cli").join(f)).ok()?)?;
        let _ = std::fs::write(&cache, &u);
        Some(u)
    })
}

/// Executes `check_antigravity_status` (check antigravity status).
pub fn check_antigravity_status(name: &str, profile: &str, screen: &str) -> (ServiceStatus, Option<String>, Option<String>, Option<u32>) {
    if !binary_exists("agy") { return (ServiceStatus::NotInstalled, None, None, None); }
    if let Some((pid, up)) = find_antigravity_process(name) {
        (ServiceStatus::Active, extract_antigravity_url(profile, screen), Some(up), Some(pid))
    } else {
        (ServiceStatus::Inactive, None, None, None)
    }
}

/// Executes `check_tailscale_status` (check tailscale status).
pub fn check_tailscale_status() -> (ServiceStatus, Option<String>) {
    if !binary_exists("tailscale") { return (ServiceStatus::NotInstalled, None); }
    if let Ok(out) = Command::new("tailscale").args(["ip", "-4"]).output() && out.status.success() {
        let ip = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if ip.starts_with("100.") { return (ServiceStatus::Active, Some(ip)); }
    }
    (ServiceStatus::Inactive, None)
}

/// Executes `check_ssh_status` (check ssh status).
pub fn check_ssh_status() -> ServiceStatus {
    if "127.0.0.1:22".parse().is_ok_and(|a| TcpStream::connect_timeout(&a, Duration::from_millis(350)).is_ok()) {
        ServiceStatus::Active
    } else if binary_exists("sshd") || binary_exists("ssh") {
        ServiceStatus::Inactive
    } else {
        ServiceStatus::NotInstalled
    }
}

/// Executes `query_remote_hub_status` (query remote hub status).
pub fn query_remote_hub_status(working_dir: &Path) -> RemoteHubStatus {
    let (profile, remote_workspace, screen, name) = resolve_profile_info(working_dir);
    let (antigravity, antigravity_url, antigravity_uptime, antigravity_pid) = check_antigravity_status(&name, &profile, &screen);
    let (tailscale, tailscale_ip) = check_tailscale_status();
    RemoteHubStatus { profile, remote_workspace, antigravity, antigravity_url, antigravity_uptime, antigravity_pid, tailscale, tailscale_ip, ssh: check_ssh_status() }
}

/// Executes `start_antigravity_remote` (start antigravity remote).
pub async fn start_antigravity_remote(working_dir: &Path) -> Result<String, String> {
    let (profile, ws_dir, screen, name) = resolve_profile_info(working_dir);
    let _ = stop_antigravity_remote(working_dir).await;
    let _ = std::fs::create_dir_all(&ws_dir);
    crate::cli::antigravity::trust::trust_workspace_in_settings(&ws_dir, None);
    ensure_remote_control_hostname(&name);

    let ws = ws_dir.display();
    let ssh = "SSH_CONNECTION='127.0.0.1 0 127.0.0.1 22'";
    let cmd = format!("{ssh} agy remote-control serve --remote-control-name {name} --add-dir {ws} || {ssh} agy --remote-control --remote-control-name {name} --add-dir {ws}");
    if binary_exists("screen") {
        if !Command::new("screen").args(["-dmS", &screen, "bash", "-c", &cmd]).current_dir(&ws_dir).status().map_err(|e| format!("Spawn error: {e}"))?.success() {
            return Err("Screen spawn failed".to_string());
        }
    } else {
        let tmp = format!("/tmp/screen_{screen}.txt");
        Command::new("setsid").args(["script", "-q", "-f", "-c", &cmd, &tmp]).current_dir(&ws_dir)
            .stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null())
            .spawn().map_err(|e| format!("Spawn error: {e}"))?;
    }
    for _ in 0..10 {
        tokio::time::sleep(Duration::from_millis(500)).await;
        if find_antigravity_process(&name).is_some() && let Some(url) = extract_antigravity_url(&profile, &screen) {
            return Ok(url);
        }
    }
    Ok("https://antigravity.google.com".to_string())
}

/// Executes `stop_antigravity_remote` (stop antigravity remote).
pub async fn stop_antigravity_remote(working_dir: &Path) -> Result<(), String> {
    let (profile, _, screen, name) = resolve_profile_info(working_dir);
    if binary_exists("screen") && find_antigravity_process(&name).is_some() {
        let _ = Command::new("screen").args(["-S", &screen, "-p", "0", "-X", "stuff", "/remote-control off\r"]).output();
        tokio::time::sleep(Duration::from_millis(1200)).await;
    }
    let pat = format!("--remote-control-name {name}");
    let _ = Command::new("pkill").args(["-15", "-f", "--", &pat]).output();
    if binary_exists("screen") {
        let _ = Command::new("screen").args(["-S", &screen, "-X", "quit"]).output();
        if profile == "default" {
            for s in ["tuner-agy-remote", "agy-tuner"] { let _ = Command::new("screen").args(["-S", s, "-X", "quit"]).output(); }
        }
    }
    if profile == "default" { let _ = Command::new("pkill").args(["-15", "-f", "--", "wimvm-dev"]).output(); }
    for f in [format!("/tmp/tuner_agy_url_{profile}.txt"), format!("/tmp/screen_{screen}.txt")] { let _ = std::fs::remove_file(f); }
    for _ in 0..10 {
        tokio::time::sleep(Duration::from_millis(200)).await;
        if find_antigravity_process(&name).is_none() { return Ok(()); }
    }
    let _ = Command::new("pkill").args(["-9", "-f", "--", &pat]).output();
    tokio::time::sleep(Duration::from_millis(150)).await;
    Ok(())
}
