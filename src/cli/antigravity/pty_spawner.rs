//! # PTY Process Spawner
//!
//! ## Overview
//! Spawns process targets wrapped in standard Unix pseudo-terminals (PTY). Manages raw I/O descriptors.
//!
//! ## Collaboration Graph
//! - Invoked by [`AntigravityCli`](super::AntigravityCli) to boot interactive agent loops.
//!
//! ## Search Tags
//! #pty-descriptor, #process-fork, #unix-io

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
    pub initialized: bool,
}

impl Drop for SessionHolder {
    fn drop(&mut self) {
        self.drain_task.abort();
        if let Some(pid) = self.child.id() {
            let pgid = nix::unistd::Pid::from_raw(-(pid as i32));
            let _ = nix::sys::signal::kill(pgid, nix::sys::signal::Signal::SIGKILL);
        }
        let _ = self.child.start_kill();
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
        initialized: false,
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
