use super::session::spawn_session;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

#[tokio::test]
async fn test_spawn_session_creates_active_process_and_cleans_up() {
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let env = HashMap::new();

    // Spawn a long-running cat process inside PTY
    let holder_res = spawn_session(&workspace, "cat", &[], &env);
    assert!(holder_res.is_ok());

    let mut holder = holder_res.unwrap();
    let pid = holder.child.id();
    assert!(pid.is_some());

    // Verify the process is currently running (no status exit yet)
    let status = holder.child.try_wait();
    assert!(status.is_ok());
    assert!(status.unwrap().is_none());

    // Wait a short time to let the drain task start
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Drop the holder, which should trigger the Drop implementation
    // causing SIGKILL to be sent to the process group
    drop(holder);

    // Verify the process is dead by trying to send SIG0 to it using nix
    let raw_pid = pid.unwrap() as i32;
    let nix_pid = nix::unistd::Pid::from_raw(raw_pid);
    
    // Give the OS a brief moment to clean it up
    tokio::time::sleep(Duration::from_millis(50)).await;

    let signal_res = nix::sys::signal::kill(nix_pid, None);
    assert!(signal_res.is_err()); // ESRCH: No such process
}

#[tokio::test]
async fn test_session_manager_manages_lifecycle() {
    use super::session::SessionManager;
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let env = HashMap::new();
    let manager = SessionManager::new();

    // 1. Ensure new session
    let res = manager.ensure_session("session-1", &workspace, "cat", &[], &env).await;
    assert!(res.is_ok());

    // 2. Verify is_active
    assert!(manager.is_active("session-1").await);

    // 3. Ensure again (idempotent, shouldn't spawn new, just reuse)
    let res2 = manager.ensure_session("session-1", &workspace, "cat", &[], &env).await;
    assert!(res2.is_ok());

    // 4. Terminate
    let terminated = manager.terminate("session-1").await;
    assert!(terminated);

    // 5. Verify no longer active
    assert!(!manager.is_active("session-1").await);
}

#[tokio::test]
async fn test_session_manager_cleans_dead_sessions() {
    use super::session::SessionManager;
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let env = HashMap::new();
    let manager = SessionManager::new();

    // Spawn a process that exits immediately (e.g. echo)
    let res = manager.ensure_session("session-quick", &workspace, "echo", &["hello".to_string()], &env).await;
    assert!(res.is_ok());

    // Wait a brief moment to let the process exit
    tokio::time::sleep(Duration::from_millis(50)).await;

    // cleanup_expired should remove it
    manager.cleanup_expired().await;

    // Verify it is no longer tracked
    assert!(!manager.is_active("session-quick").await);
}

#[tokio::test]
async fn test_session_manager_terminate_all_kills_descendants() {
    use super::session::SessionManager;
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let env = HashMap::new();
    let manager = SessionManager::new();

    // 1. Spawn two long-running sessions
    let res1 = manager.ensure_session("sess-term-1", &workspace, "cat", &[], &env).await;
    let res2 = manager.ensure_session("sess-term-2", &workspace, "cat", &[], &env).await;
    assert!(res1.is_ok());
    assert!(res2.is_ok());

    // 2. Fetch process IDs manually to verify death later
    let pid1 = {
        let holders = manager.holders.lock().await;
        holders.get("sess-term-1").unwrap().child.id().unwrap()
    };
    let pid2 = {
        let holders = manager.holders.lock().await;
        holders.get("sess-term-2").unwrap().child.id().unwrap()
    };

    // Verify both are running
    assert!(manager.is_active("sess-term-1").await);
    assert!(manager.is_active("sess-term-2").await);

    // 3. Terminate all
    manager.terminate_all().await;

    // Give OS a small moment
    tokio::time::sleep(Duration::from_millis(50)).await;

    // 4. Verify both are dead using kill(pid, None) -> ESRCH
    let nix_pid1 = nix::unistd::Pid::from_raw(pid1 as i32);
    let nix_pid2 = nix::unistd::Pid::from_raw(pid2 as i32);
    assert!(nix::sys::signal::kill(nix_pid1, None).is_err());
    assert!(nix::sys::signal::kill(nix_pid2, None).is_err());
}

