//! # Subprocess Worker Supervisor
//!
//! ## Overview
//! Monitors child subprocesses, restarts workers on failure, and handles termination signals.
//!
//! ## Search Tags
//! #supervisor, #subprocess-runner, #signal-propagator

use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokio::process::Command;

/// Represents `Supervisor` (supervisor).
pub struct Supervisor {
    child_path: PathBuf,
    args: Vec<String>,
}

impl Supervisor {
    /// Executes `with_args` (with args).
    pub fn with_args(child_path: PathBuf, args: Vec<String>) -> Self {
        Self { child_path, args }
    }

    /// Executes `run` (run).
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
            let backoff = Duration::from_secs_f64(f64::min(2.0f64.powi(fast_crash_count), 30.0));
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
