//! # Antigravity Session Holder and PTY Spawning
//!
//! This module manages active interactive sessions with the Google Antigravity CLI.
//! It handles non-echoing PTY spawning, non-blocking asynchronous output draining,
//! and automatic resource cleanup to prevent zombie processes.

use nix::fcntl::{fcntl, FcntlArg, OFlag};
use nix::pty::openpty;
use nix::sys::termios::{tcgetattr, tcsetattr, LocalFlags, SetArg};
use std::collections::HashMap;
use std::os::unix::io::{AsFd, AsRawFd};
use std::path::Path;
use std::process::Stdio;
use std::time::Instant;
use tokio::io::unix::AsyncFd;
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

pub struct SessionHolder {
    pub child: Child,
    pub drain_task: JoinHandle<()>,
    pub last_active: Instant,
    pub chat_id: Option<i64>,
    pub topic_id: Option<i64>,
    pub master_fd: std::os::unix::io::RawFd,
    pub output: std::sync::Arc<Mutex<Vec<u8>>>,
}

impl Drop for SessionHolder {
    fn drop(&mut self) {
        // Abort background reader task to prevent resource leakage
        self.drain_task.abort();

        // Kill the whole process group to clean up descendant processes
        if let Some(pid) = self.child.id() {
            let pgid = nix::unistd::Pid::from_raw(-(pid as i32));
            let _ = nix::sys::signal::kill(pgid, nix::sys::signal::Signal::SIGKILL);
        }

        // Kill child process directly
        let _ = self.child.start_kill();

        // Close duplicate master fd
        let _ = nix::unistd::close(self.master_fd);
    }
}

impl SessionHolder {
    pub fn write_input(&self, input: &str) -> Result<(), String> {
        let mut bytes_written = 0;
        let data = input.as_bytes();
        while bytes_written < data.len() {
            match nix::unistd::write(self.master_fd, &data[bytes_written..]) {
                Ok(n) => {
                    if n == 0 {
                        return Err("Written 0 bytes (pipe closed?)".to_string());
                    }
                    bytes_written += n;
                }
                Err(nix::Error::EINTR) => {}
                Err(e) => return Err(e.to_string()),
            }
        }
        Ok(())
    }
}

fn disable_echo<Fd: AsFd>(fd: Fd) -> Result<(), String> {
    if let Ok(mut termios) = tcgetattr(&fd) {
        termios.local_flags.remove(LocalFlags::ECHO);
        tcsetattr(&fd, SetArg::TCSANOW, &termios).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn set_non_blocking<Fd: AsFd>(fd: Fd) -> Result<(), String> {
    let raw_fd = fd.as_fd().as_raw_fd();
    let flags = fcntl(raw_fd, FcntlArg::F_GETFL).map_err(|e| e.to_string())?;
    let mut oflags = OFlag::from_bits_truncate(flags);
    oflags.insert(OFlag::O_NONBLOCK);
    fcntl(raw_fd, FcntlArg::F_SETFL(oflags)).map_err(|e| e.to_string())?;
    Ok(())
}

/// Spawns a command inside a new PTY session with echo disabled.
pub fn spawn_session(
    workspace: &Path,
    cmd_name: &str,
    args: &[String],
    env: &HashMap<String, String>,
) -> Result<SessionHolder, String> {
    let pty = openpty(None, None).map_err(|e| e.to_string())?;

    disable_echo(&pty.slave)?;
    set_non_blocking(&pty.master)?;

    let master_raw = pty.master.as_raw_fd();
    let master_dup = nix::unistd::dup(master_raw).map_err(|e| e.to_string())?;

    let stdin_redirect = Stdio::from(pty.slave.try_clone().map_err(|e| e.to_string())?);
    let stdout_redirect = Stdio::from(pty.slave.try_clone().map_err(|e| e.to_string())?);
    let stderr_redirect = Stdio::from(pty.slave);

    let mut cmd = Command::new(cmd_name);
    cmd.args(args)
        .current_dir(workspace)
        .envs(env)
        .stdin(stdin_redirect)
        .stdout(stdout_redirect)
        .stderr(stderr_redirect)
        .process_group(0)
        .kill_on_drop(true);

    let child = cmd.spawn().map_err(|e| e.to_string())?;
    let output = std::sync::Arc::new(Mutex::new(Vec::new()));
    let async_master = AsyncFd::new(pty.master).map_err(|e| e.to_string())?;
    let drain_task = spawn_drain_task(async_master, output.clone());

    let chat_id = env.get("TUNER_CHAT_ID").and_then(|s| s.parse::<i64>().ok());
    let topic_id = env.get("TUNER_TOPIC_ID").and_then(|s| s.parse::<i64>().ok());

    Ok(SessionHolder {
        child,
        drain_task,
        last_active: Instant::now(),
        chat_id,
        topic_id,
        master_fd: master_dup,
        output,
    })
}

fn spawn_drain_task(
    async_master: AsyncFd<std::os::fd::OwnedFd>,
    output: std::sync::Arc<Mutex<Vec<u8>>>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut buf = [0u8; 4096];
        loop {
            match async_master.readable().await {
                Ok(mut guard) => {
                    match nix::unistd::read(async_master.get_ref().as_raw_fd(), &mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            let mut out = output.lock().await;
                            out.extend_from_slice(&buf[..n]);
                            guard.clear_ready();
                        }
                        Err(nix::Error::EAGAIN) => {
                            guard.clear_ready();
                        }
                        Err(_) => break,
                    }
                }
                Err(_) => break,
            }
        }
    })
}

pub struct SessionManager {
    pub(crate) holders: Mutex<HashMap<String, SessionHolder>>,
    pub(crate) running_runs: Mutex<std::collections::HashSet<String>>,
    pub(crate) active_asks: Mutex<std::collections::HashSet<String>>,
    pub(crate) active_ask_options: Mutex<HashMap<String, (i32, Vec<String>)>>,
}

impl Default for SessionManager {
    fn default() -> Self {
        Self {
            holders: Mutex::new(HashMap::new()),
            running_runs: Mutex::new(std::collections::HashSet::new()),
            active_asks: Mutex::new(std::collections::HashSet::new()),
            active_ask_options: Mutex::new(HashMap::new()),
        }
    }
}

fn terminate_duplicates(
    sid: &str,
    cid: Option<i64>,
    tid: Option<i64>,
    holders: &mut HashMap<String, SessionHolder>,
) {
    if let Some(c) = cid {
        holders.retain(|id, h| id == sid || h.chat_id != Some(c) || h.topic_id != tid);
    }
}

impl SessionManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn is_running(&self, session_id: &str) -> bool {
        let runs = self.running_runs.lock().await;
        runs.contains(session_id)
    }

    pub async fn set_running(&self, session_id: &str, running: bool) {
        let mut runs = self.running_runs.lock().await;
        if running {
            runs.insert(session_id.to_string());
        } else {
            runs.remove(session_id);
        }
    }

    pub async fn is_ask_active(&self, session_id: &str) -> bool {
        let asks = self.active_asks.lock().await;
        asks.contains(session_id)
    }

    pub async fn set_ask_active(&self, session_id: &str, active: bool) {
        let mut asks = self.active_asks.lock().await;
        if active {
            asks.insert(session_id.to_string());
        } else {
            asks.remove(session_id);
            self.active_ask_options.lock().await.remove(session_id);
        }
    }

    pub async fn set_ask_data(&self, session_id: &str, msg_id: i32, options: Vec<String>) {
        self.active_ask_options.lock().await.insert(session_id.to_string(), (msg_id, options));
    }

    pub async fn get_ask_options(&self, session_id: &str) -> Option<Vec<String>> {
        self.active_ask_options.lock().await.get(session_id).map(|(_, v)| v.clone())
    }

    pub async fn get_ask_msg_id(&self, session_id: &str) -> Option<i32> {
        self.active_ask_options.lock().await.get(session_id).map(|(id, _)| *id)
    }

    pub async fn cleanup_expired(&self) {
        let now = Instant::now();
        self.holders.lock().await.retain(|_, h| {
            h.child.try_wait().map(|s| s.is_none()).unwrap_or(false)
                && now.duration_since(h.last_active) < std::time::Duration::from_secs(86400)
        });
    }

    pub async fn ensure_session(
        &self,
        session_id: &str,
        workspace: &Path,
        cmd_name: &str,
        cmd_args: &[String],
        env: &HashMap<String, String>,
    ) -> Result<(), String> {
        self.cleanup_expired().await;

        let chat_id = env.get("TUNER_CHAT_ID").and_then(|s| s.parse::<i64>().ok());
        let topic_id = env.get("TUNER_TOPIC_ID").and_then(|s| s.parse::<i64>().ok());

        let mut holders = self.holders.lock().await;
        let is_running = holders.get_mut(session_id)
            .map(|h| h.child.try_wait().map(|s| s.is_none()).unwrap_or(false))
            .unwrap_or(false);

        if is_running {
            terminate_duplicates(session_id, chat_id, topic_id, &mut holders);
            if let Some(h) = holders.get_mut(session_id) { h.last_active = Instant::now(); }
            return Ok(());
        }
        holders.remove(session_id);

        terminate_duplicates(session_id, chat_id, topic_id, &mut holders);

        let holder = spawn_session(workspace, cmd_name, cmd_args, env)?;
        holders.insert(session_id.to_string(), holder);
        Ok(())
    }

    pub async fn terminate(&self, session_id: &str) -> bool {
        let mut holders = self.holders.lock().await;
        holders.remove(session_id).is_some()
    }

    pub async fn abort(&self, chat_id: i64, topic_id: Option<i64>) -> usize {
        let mut holders = self.holders.lock().await;
        let prev_len = holders.len();
        holders.retain(|_, h| h.chat_id != Some(chat_id) || h.topic_id != topic_id);
        prev_len - holders.len()
    }

    pub async fn terminate_all(&self) {
        let mut holders = self.holders.lock().await;
        holders.clear();
    }

    pub async fn write_to_session(&self, session_id: &str, input: &str) -> Result<bool, String> {
        let holders = self.holders.lock().await;
        if let Some(holder) = holders.get(session_id) {
            holder.write_input(input)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub async fn is_active(&self, session_id: &str) -> bool {
        let mut h = self.holders.lock().await;
        if let Some(holder) = h.get_mut(session_id) {
            if holder.child.try_wait().map(|s| s.is_none()).unwrap_or(false) { return true; }
            h.remove(session_id);
        }
        false
    }
}

