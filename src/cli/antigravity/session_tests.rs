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

#[tokio::test]
async fn test_session_manager_terminates_duplicate_chat_sessions() {
    use super::session::SessionManager;
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut env = HashMap::new();
    env.insert("TUNER_CHAT_ID".to_string(), "100".to_string());
    env.insert("TUNER_TOPIC_ID".to_string(), "200".to_string());
    let manager = SessionManager::new();

    // 1. Spawning session-1
    let res1 = manager.ensure_session("session-1", &workspace, "cat", &[], &env).await;
    assert!(res1.is_ok());
    assert!(manager.is_active("session-1").await);

    // 2. Spawning session-2 with the same chat/topic keys should terminate session-1
    let res2 = manager.ensure_session("session-2", &workspace, "cat", &[], &env).await;
    assert!(res2.is_ok());

    // Verify session-2 is active, but session-1 is terminated
    assert!(manager.is_active("session-2").await);
    assert!(!manager.is_active("session-1").await);
}

#[tokio::test]
async fn test_write_to_session_does_not_block_concurrent_holder_access() {
    use super::session::SessionManager;
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let env = HashMap::new();
    let manager = std::sync::Arc::new(SessionManager::new());

    // 1. Ensure a session
    let res = manager.ensure_session("sess-write-bench", &workspace, "cat", &[], &env).await;
    assert!(res.is_ok());

    // 2. Start a paced write in the background with backspace erasure characters
    let mgr_clone = manager.clone();
    let write_handle = tokio::spawn(async move {
        let long_input = "\x7f".repeat(50);
        mgr_clone.write_to_session("sess-write-bench", &long_input).await
    });

    // 3. Immediately verify that is_active and ensure_session on another key do NOT block
    tokio::time::sleep(Duration::from_millis(5)).await;

    let start = std::time::Instant::now();
    let is_act = manager.is_active("sess-write-bench").await;
    assert!(is_act);

    let res2 = manager.ensure_session("sess-write-concurrent", &workspace, "cat", &[], &env).await;
    assert!(res2.is_ok());
    let elapsed = start.elapsed();

    // The concurrent operations must complete in less than 35ms without being blocked
    assert!(elapsed < Duration::from_millis(35), "Concurrent access took too long: {:?}", elapsed);

    let write_res = write_handle.await.unwrap();
    assert!(write_res.is_ok());

    // Clean up
    manager.terminate_all().await;
}

#[tokio::test]
async fn test_write_to_session_bulk_prompt_instantaneous() {
    use super::session::SessionManager;
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let env = HashMap::new();
    let manager = SessionManager::new();

    let res = manager.ensure_session("sess-bulk-test", &workspace, "cat", &[], &env).await;
    assert!(res.is_ok());

    // 2,000 characters prompt (typical large user prompt / code question)
    let large_prompt = "x".repeat(2000);
    let start = std::time::Instant::now();
    let write_res = manager.write_to_session("sess-bulk-test", &large_prompt).await;
    let elapsed = start.elapsed();

    assert!(write_res.is_ok());
    assert!(write_res.unwrap());
    // With 0ms bulk writes, 2,000 chars must complete in less than 15ms!
    assert!(elapsed < Duration::from_millis(15), "Bulk prompt write took too long: {:?}", elapsed);

    manager.terminate("sess-bulk-test").await;
}

