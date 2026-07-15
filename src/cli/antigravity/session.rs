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

fn create_command(
    cmd_name: &str,
    args: &[String],
    env: &HashMap<String, String>,
    workspace: &Path,
    stdin: Stdio,
    stdout: Stdio,
    stderr: Stdio,
) -> Command {
    let mut cmd = Command::new(cmd_name);
    cmd.args(args)
        .current_dir(workspace)
        .envs(env)
        .stdin(stdin)
        .stdout(stdout)
        .stderr(stderr)
        .process_group(0)
        .kill_on_drop(true);
    cmd
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

    let stdin_redirect = Stdio::from(pty.slave.try_clone().map_err(|e| e.to_string())?);
    let stdout_redirect = Stdio::from(pty.slave.try_clone().map_err(|e| e.to_string())?);
    let stderr_redirect = Stdio::from(pty.slave);

    let mut cmd = create_command(
        cmd_name,
        args,
        env,
        workspace,
        stdin_redirect,
        stdout_redirect,
        stderr_redirect,
    );
    let child = cmd.spawn().map_err(|e| e.to_string())?;

    let async_master = AsyncFd::new(pty.master).map_err(|e| e.to_string())?;
    let drain_task = tokio::spawn(async move {
        let mut buf = [0u8; 4096];
        loop {
            match async_master.readable().await {
                Ok(mut guard) => {
                    match nix::unistd::read(async_master.get_ref().as_raw_fd(), &mut buf) {
                        Ok(0) => break,
                        Ok(_) => guard.clear_ready(),
                        Err(nix::Error::EAGAIN) => {
                            guard.clear_ready();
                        }
                        Err(_) => break,
                    }
                }
                Err(_) => break,
            }
        }
    });

    let chat_id = env.get("TUNER_CHAT_ID").and_then(|s| s.parse::<i64>().ok());
    let topic_id = env.get("TUNER_TOPIC_ID").and_then(|s| s.parse::<i64>().ok());

    Ok(SessionHolder {
        child,
        drain_task,
        last_active: Instant::now(),
        chat_id,
        topic_id,
    })
}

#[derive(Default)]
pub struct SessionManager {
    pub(crate) holders: Mutex<HashMap<String, SessionHolder>>,
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

    pub async fn cleanup_expired(&self) {
        let mut holders = self.holders.lock().await;
        let mut expired = Vec::new();
        let now = Instant::now();

        for (id, holder) in holders.iter_mut() {
            let is_dead = match holder.child.try_wait() {
                Ok(Some(_)) => true,
                Err(_) => true,
                Ok(None) => false,
            };
            let is_expired = now.duration_since(holder.last_active) > std::time::Duration::from_secs(86400);
            if is_dead || is_expired {
                expired.push(id.clone());
            }
        }

        for id in expired {
            holders.remove(&id);
        }
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

