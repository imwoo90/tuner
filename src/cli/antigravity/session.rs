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
}

impl Default for SessionManager {
    fn default() -> Self {
        Self {
            holders: Mutex::new(HashMap::new()),
            running_runs: Mutex::new(std::collections::HashSet::new()),
        }
    }
}

fn terminate_duplicates(
    session_id: &str,
    chat_id: Option<i64>,
    topic_id: Option<i64>,
    holders: &mut HashMap<String, SessionHolder>,
) {
    if let Some(cid) = chat_id {
        let mut keys_to_remove = Vec::new();
        for (id, h) in holders.iter_mut() {
            if id != session_id && h.chat_id == Some(cid) && h.topic_id == topic_id {
                keys_to_remove.push(id.clone());
            }
        }
        for id in keys_to_remove {
            holders.remove(&id);
        }
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

    pub async fn cleanup_expired(&self) {
        let mut holders = self.holders.lock().await;
        let now = Instant::now();
        holders.retain(|_, h| {
            let is_dead = h.child.try_wait().map(|s| s.is_some()).unwrap_or(true);
            let is_fresh = now.duration_since(h.last_active) < std::time::Duration::from_secs(86400);
            !is_dead && is_fresh
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
        let is_running = if let Some(holder) = holders.get_mut(session_id) {
            match holder.child.try_wait() {
                Ok(None) => true,
                _ => false,
            }
        } else {
            false
        };

        if is_running {
            // Terminate any other old sessions for the same chat/topic
            terminate_duplicates(session_id, chat_id, topic_id, &mut holders);
            if let Some(holder) = holders.get_mut(session_id) {
                holder.last_active = Instant::now();
            }
            return Ok(());
        } else {
            holders.remove(session_id);
        }

        // Terminate any old sessions for the same chat/topic before spawning the new one
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
        let mut to_remove = Vec::new();
        for (id, holder) in holders.iter() {
            if holder.chat_id == Some(chat_id) && holder.topic_id == topic_id {
                to_remove.push(id.clone());
            }
        }
        let count = to_remove.len();
        for id in to_remove {
            holders.remove(&id);
        }
        count
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
        let mut holders = self.holders.lock().await;
        if let Some(holder) = holders.get_mut(session_id) {
            match holder.child.try_wait() {
                Ok(None) => true,
                _ => {
                    holders.remove(session_id);
                    false
                }
            }
        } else {
            false
        }
    }
}

