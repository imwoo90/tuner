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

pub fn write_fd(fd: std::os::unix::io::RawFd, input: &str) -> Result<(), String> {
    let mut bytes_written = 0;
    let data = input.as_bytes();
    while bytes_written < data.len() {
        match nix::unistd::write(fd, &data[bytes_written..]) {
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

impl SessionHolder {
    pub fn write_input(&self, input: &str) -> Result<(), String> {
        write_fd(self.master_fd, input)
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
    let winsize = nix::pty::Winsize {
        ws_row: 24,
        ws_col: 80,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let pty = openpty(Some(&winsize), None).map_err(|e| e.to_string())?;

    disable_echo(&pty.slave)?;
    set_non_blocking(&pty.master)?;

    let master_raw = pty.master.as_raw_fd();
    let slave_raw = pty.slave.as_raw_fd();
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

    unsafe {
        cmd.pre_exec(move || {
            let _ = nix::unistd::setsid();
            let _ = nix::unistd::tcsetpgrp(slave_raw, nix::unistd::getpid());
            Ok(())
        });
    }

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

pub(crate) fn strip_ansi(s: &str) -> String {
    static ANSI_RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = ANSI_RE.get_or_init(|| regex::Regex::new(r"\x1B(?:\[[0-9;?]*[a-zA-Z=hlm]|[\(\)][a-zA-Z0-9])").unwrap());
    re.replace_all(s, "").to_string()
}

pub(crate) async fn wait_for_pty_prompt(mgr: &super::session::SessionManager, sid: &str) -> Result<(), String> {
    let start = std::time::Instant::now();
    while start.elapsed().as_secs() < 15 {
        let mut hs = mgr.holders.lock().await;
        let h = hs.get_mut(sid).ok_or("No holder")?;
        let out = h.output.lock().await.clone();
        let dead = h.child.try_wait().ok().flatten().is_some();
        let s = String::from_utf8_lossy(&out);
        let clean = strip_ansi(&s);
        if clean.contains('>') || dead {
            drop(hs);
            tokio::time::sleep(tokio::time::Duration::from_millis(4000)).await;
            return Ok(());
        }
        drop(hs);
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }
    let last_out = if let Some(h) = mgr.holders.lock().await.get(sid) {
        String::from_utf8_lossy(&h.output.lock().await).to_string()
    } else {
        "None".into()
    };
    Err(format!("Timeout waiting for session init. PTY: {}", last_out))
}
