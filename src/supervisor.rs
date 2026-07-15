//! Supervisor module to manage the lifecycle of a child process.
//! Implements auto-restart on exit code 42 and exponential backoff for crashes.

use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokio::process::Command;

pub struct Supervisor {
    child_path: PathBuf,
    args: Vec<String>,
}

impl Supervisor {
    pub fn new(child_path: PathBuf) -> Self {
        Self {
            child_path,
            args: Vec::new(),
        }
    }

    pub fn with_args(child_path: PathBuf, args: Vec<String>) -> Self {
        Self { child_path, args }
    }

    pub async fn run(&self) -> Result<(), String> {
        let mut fast_crash_count = 0;
        loop {
            let (exit_code, runtime) = self.spawn_and_wait().await?;
            if exit_code == 0 {
                break;
            }
            if exit_code == 42 {
                fast_crash_count = 0;
                continue;
            }
            if runtime < Duration::from_secs(10) {
                fast_crash_count += 1;
            } else {
                fast_crash_count = 0;
            }
            let backoff = Duration::from_secs_f64(f64::min(2.0f64.powi(fast_crash_count as i32), 30.0));
            tokio::time::sleep(backoff).await;
        }
        Ok(())
    }

    async fn spawn_and_wait(&self) -> Result<(i32, Duration), String> {
        let start = Instant::now();
        let mut child = Command::new(&self.child_path)
            .args(&self.args)
            .env("TUNER_SUPERVISOR", "1")
            .spawn()
            .map_err(|e| format!("Spawn failed: {}", e))?;

        let status = child.wait().await.map_err(|e| format!("Wait failed: {}", e))?;
        Ok((status.code().unwrap_or(-1), start.elapsed()))
    }
}

pub async fn terminate_child(child: &mut tokio::process::Child, timeout: Duration) -> Result<(), std::io::Error> {
    if child.try_wait()?.is_some() {
        return Ok(());
    }

    #[cfg(unix)]
    {
        if let Some(pid) = child.id() {
            let pid = nix::unistd::Pid::from_raw(pid as i32);
            let _ = nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGTERM);
        }
    }
    #[cfg(not(unix))]
    {
        let _ = child.kill().await;
    }

    match tokio::time::timeout(timeout, child.wait()).await {
        Ok(res) => {
            res?;
            Ok(())
        }
        Err(_) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            Ok(())
        }
    }
}
