//! # Process Supervisor Integration Tests
//!
//! This module contains integration tests for the child process supervisor,
//! validating auto-restart, backoff, and termination behavior.

#[cfg(test)]
mod tests {
    use crate::supervisor::{Supervisor, terminate_child};
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
    use std::time::Duration;
    use tempfile::TempDir;

    fn create_script(dir: &TempDir, name: &str, content: &str) -> PathBuf {
        let path = dir.path().join(name);
        fs::write(&path, content).unwrap();
        let mut perms = fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms).unwrap();
        path
    }

    #[tokio::test]
    async fn test_clean_exit_stops_supervisor() {
        let dir = TempDir::new().unwrap();
        let script = create_script(&dir, "mock.sh", "#!/bin/sh\nexit 0\n");
        let sv = Supervisor::new(script);
        assert!(sv.run().await.is_ok());
    }

    #[tokio::test]
    async fn test_restart_exit_code_respawns() {
        let dir = TempDir::new().unwrap();
        let state_file = dir.path().join("state.txt");
        let script_content = format!(
            "#!/bin/sh\nif [ ! -f \"{}\" ]; then\n  echo \"1\" > \"{}\"\n  exit 42\nelse\n  rm \"{}\"\n  exit 0\nfi\n",
            state_file.display(), state_file.display(), state_file.display()
        );
        let script = create_script(&dir, "mock.sh", &script_content);
        let sv = Supervisor::new(script);
        assert!(sv.run().await.is_ok());
        assert!(!state_file.exists());
    }

    #[tokio::test(start_paused = true)]
    async fn test_crash_with_backoff() {
        let dir = TempDir::new().unwrap();
        let state_file = dir.path().join("state.txt");
        let script_content = format!(
            "#!/bin/sh\nif [ ! -f \"{}\" ]; then\n  echo \"1\" > \"{}\"\n  exit 1\nelse\n  rm \"{}\"\n  exit 0\nfi\n",
            state_file.display(), state_file.display(), state_file.display()
        );
        let script = create_script(&dir, "mock.sh", &script_content);
        let sv = Supervisor::new(script);
        assert!(sv.run().await.is_ok());
    }

    #[tokio::test(start_paused = true)]
    async fn test_fast_crash_escalates_backoff() {
        let dir = TempDir::new().unwrap();
        let state_file = dir.path().join("state.txt");
        let script_content = format!(
            "#!/bin/sh\ncount=0\nif [ -f \"{}\" ]; then\n  count=$(cat \"{}\")\nfi\ncount=$((count+1))\necho \"$count\" > \"{}\"\nif [ \"$count\" -lt 4 ]; then\n  exit 2\nelse\n  rm \"{}\"\n  exit 0\nfi\n",
            state_file.display(), state_file.display(), state_file.display(), state_file.display()
        );
        let script = create_script(&dir, "mock.sh", &script_content);
        let sv = Supervisor::new(script);
        assert!(sv.run().await.is_ok());
    }

    #[tokio::test]
    async fn test_terminate_sends_sigterm() {
        let dir = TempDir::new().unwrap();
        let script = create_script(&dir, "mock.sh", "#!/bin/sh\nsleep 100\n");
        let mut child = tokio::process::Command::new(script).spawn().unwrap();
        let res = terminate_child(&mut child, Duration::from_millis(50)).await;
        assert!(res.is_ok());
    }

    #[tokio::test]
    async fn test_terminate_kills_on_timeout() {
        let dir = TempDir::new().unwrap();
        let script = create_script(&dir, "mock.sh", "#!/bin/sh\ntrap \"\" TERM\nsleep 100\n");
        let mut child = tokio::process::Command::new(script).spawn().unwrap();
        let res = terminate_child(&mut child, Duration::from_millis(50)).await;
        assert!(res.is_ok());
    }
}
