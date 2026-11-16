//! # Xvfb & Chrome Persistent Profile Process Runtime (`browser.rs`)
//!
//! ## Overview
//! Launches an isolated `Xvfb -displayfd 1` X11 framebuffer and headed Chrome process group
//! using [`super::browser_env`], awaits `DevToolsActivePort`, and enforces zero-orphan teardown
//! via process-group signals and a safe parent-PID watchdog.
//!
//! ## Search Tags
//! #xvfb-launcher, #chrome-cdp-spawner, #persistent-profile, #orphan-reaper

use super::browser_env::{needs_no_sandbox, prepare_profile_dir, resolve_browser_binary};
use crate::cli::common::kill_process_group;
use crate::setup::doctor::which_bin;
use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

/// Viewport width in CSS pixels optimized for mobile portrait thumb ergonomics while keeping desktop layout.
pub const VIEWPORT_WIDTH: u32 = 760;
/// Viewport height in CSS pixels for the remote authentication canvas.
pub const VIEWPORT_HEIGHT: u32 = 980;

/// Active `Xvfb` + Chrome child process handles and discovered CDP WebSocket endpoint.
pub struct BrowserRuntime {
    xvfb_child: Child,
    xvfb_pid: Option<u32>,
    chrome_child: Child,
    chrome_pid: Option<u32>,
    watchdog_child: Option<Child>,
    /// Local loopback CDP HTTP port (`127.0.0.1:<cdp_port>`).
    pub cdp_port: u16,
    /// Browser-level CDP WebSocket URL (`ws://127.0.0.1:<cdp_port>/devtools/browser/<id>`).
    pub browser_ws_url: String,
}

async fn spawn_xvfb() -> Result<(Child, String), String> {
    let xvfb_bin = which_bin("Xvfb").ok_or_else(|| {
        "Xvfb not found in PATH. Run `tuner --doctor --fix` or install `xvfb`.".to_string()
    })?;
    let mut cmd = Command::new(xvfb_bin);
    cmd.args([
        "-displayfd",
        "1",
        "-screen",
        "0",
        "800x1100x24",
        "-nolisten",
        "tcp",
        "-ac",
    ])
    .stdout(Stdio::piped())
    .stderr(Stdio::null())
    .process_group(0)
    .kill_on_drop(true);

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Failed to spawn Xvfb: {e}"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Failed to capture Xvfb stdout".to_string())?;
    let mut line = String::new();
    let res =
        tokio::time::timeout(Duration::from_secs(6), BufReader::new(stdout).read_line(&mut line))
            .await;
    if let Ok(Ok(n)) = res
        && n > 0
    {
        let disp = line.trim();
        if !disp.is_empty() && disp.chars().all(|c| c.is_ascii_digit()) {
            return Ok((child, format!(":{disp}")));
        }
    }
    let _ = child.kill().await;
    Err("Timed out or failed waiting for Xvfb -displayfd readiness".to_string())
}

fn spawn_chrome_process(
    chrome_bin: &Path,
    display: &str,
    profile_dir: &Path,
    initial_url: &str,
) -> Result<Child, String> {
    let mut cmd = Command::new(chrome_bin);
    cmd.env("DISPLAY", display)
        .env("XDG_CACHE_HOME", profile_dir.join(".xdg_cache"))
        .env("DBUS_SESSION_BUS_ADDRESS", "/dev/null")
        .args([
            "--disable-blink-features=AutomationControlled",
            "--password-store=basic",
            "--use-mock-keychain",
            "--disable-dev-shm-usage",
            "--no-first-run",
            "--no-default-browser-check",
            "--disable-search-engine-choice-screen",
            "--disable-infobars",
            "--disable-popup-blocking",
            "--allow-file-access-from-files=false",
            "--lang=ko-KR",
            "--remote-debugging-port=0",
            "--remote-debugging-address=127.0.0.1",
            &format!("--window-size={},{}", VIEWPORT_WIDTH, VIEWPORT_HEIGHT + 90),
            &format!("--user-data-dir={}", profile_dir.display()),
        ]);
    if needs_no_sandbox() {
        cmd.arg("--no-sandbox");
    }
    cmd.arg(initial_url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("Failed to launch Chrome ({:?}): {}", chrome_bin, e))
}

async fn await_devtools_port(profile_dir: &Path) -> Result<(u16, String), String> {
    let active_port_file = profile_dir.join("DevToolsActivePort");
    let start = Instant::now();
    while start.elapsed() <= Duration::from_secs(12) {
        if let Ok(raw) = std::fs::read_to_string(&active_port_file) {
            let mut lines = raw.lines();
            if let (Some(p_str), Some(path_str)) = (lines.next(), lines.next())
                && let Ok(port) = p_str.trim().parse::<u16>()
                && port > 0
                && path_str.trim().starts_with('/')
            {
                return Ok((port, path_str.trim().to_string()));
            }
        }
        tokio::time::sleep(Duration::from_millis(120)).await;
    }
    Err("Timed out waiting for Chrome DevToolsActivePort".to_string())
}

fn spawn_parent_death_watchdog(pgids: &[u32]) -> Option<Child> {
    let ppid = std::process::id();
    let targets: Vec<String> = pgids.iter().map(|p| format!("-{p}")).collect();
    if targets.is_empty() {
        return None;
    }
    let grp_list = targets.join(" ");
    let script = format!(
        "while kill -0 {ppid} 2>/dev/null; do sleep 1; done; \
         kill -TERM {grp_list} 2>/dev/null || true; sleep 1; \
         kill -KILL {grp_list} 2>/dev/null || true"
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

impl BrowserRuntime {
    /// Launches `Xvfb` + headed Chrome against `profile_dir` and awaits CDP readiness.
    pub async fn launch(profile_dir: &Path, initial_url: &str) -> Result<Self, String> {
        prepare_profile_dir(profile_dir)?;
        let chrome_bin = resolve_browser_binary()?;
        let (mut xvfb_child, display) = spawn_xvfb().await?;
        let xvfb_pid = xvfb_child.id();

        let mut chrome_child =
            match spawn_chrome_process(&chrome_bin, &display, profile_dir, initial_url) {
                Ok(c) => c,
                Err(e) => {
                    if let Some(pid) = xvfb_pid {
                        kill_process_group(pid);
                    }
                    let _ = xvfb_child.kill().await;
                    return Err(e);
                }
            };
        let chrome_pid = chrome_child.id();

        let (cdp_port, ws_path) = match await_devtools_port(profile_dir).await {
            Ok(v) => v,
            Err(e) => {
                if let Some(pid) = chrome_pid {
                    kill_process_group(pid);
                }
                if let Some(pid) = xvfb_pid {
                    kill_process_group(pid);
                }
                let _ = chrome_child.kill().await;
                let _ = xvfb_child.kill().await;
                return Err(e);
            }
        };

        let pgids: Vec<u32> = [chrome_pid, xvfb_pid].into_iter().flatten().collect();
        let watchdog_child = spawn_parent_death_watchdog(&pgids);

        Ok(Self {
            xvfb_child,
            xvfb_pid,
            chrome_child,
            chrome_pid,
            watchdog_child,
            cdp_port,
            browser_ws_url: format!("ws://127.0.0.1:{cdp_port}{ws_path}"),
        })
    }

    /// Terminates Chrome, `Xvfb`, and the parent-death watchdog cleanly.
    pub async fn shutdown(&mut self) {
        if let Some(mut wd) = self.watchdog_child.take() {
            let _ = wd.kill().await;
            let _ = wd.wait().await;
        }
        let _ = tokio::time::timeout(Duration::from_millis(1500), self.chrome_child.wait()).await;
        if let Some(pid) = self.chrome_pid {
            kill_process_group(pid);
        }
        let _ = self.chrome_child.kill().await;
        let _ = self.chrome_child.wait().await;

        if let Some(pid) = self.xvfb_pid {
            kill_process_group(pid);
        }
        let _ = self.xvfb_child.kill().await;
        let _ = self.xvfb_child.wait().await;
    }
}
