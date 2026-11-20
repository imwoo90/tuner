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

#[tokio::test]
async fn test_session_manager_reuses_running_process() {
    use super::session::SessionManager;
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let env = HashMap::new();
    let manager = SessionManager::new();

    let args1 = vec!["10".to_string()];
    manager.ensure_session("session-reuse", &workspace, "sleep", &args1, &env).await.unwrap();

    let initial_pid = {
        let h = manager.holders.lock().await;
        h.get("session-reuse").unwrap().child.id().unwrap()
    };

    manager.ensure_session("session-reuse", &workspace, "sleep", &args1, &env).await.unwrap();
    let reused_pid = {
        let h = manager.holders.lock().await;
        h.get("session-reuse").unwrap().child.id().unwrap()
    };
    assert_eq!(initial_pid, reused_pid);

    manager.terminate("session-reuse").await;
}

#[test]
fn test_has_ready_status_bar_distinguishes_unauthed_and_ready_states() {
    use super::pty_spawner::has_ready_status_bar;

    let unauthed = "────────────────────────────────\n>\n────────────────────────────────\n  ? for shortcuts\r\n";
    assert!(!has_ready_status_bar(unauthed));

    let ready = "────────────────────────────────\n>\n────────────────────────────────\n  ? for shortcuts                                 Gemini 4 Argon · high\r\n";
    assert!(has_ready_status_bar(ready));
}

#[tokio::test]
async fn test_wait_for_pty_prompt_ignores_early_csi_and_unauthed_bar() {
    use super::pty_spawner::wait_for_pty_prompt;
    use super::session::SessionManager;

    let ws = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mgr = std::sync::Arc::new(SessionManager::new());
    mgr.ensure_session("sess-ready", &ws, "cat", &[], &HashMap::new()).await.unwrap();

    {
        let hs = mgr.holders.lock().await;
        let mut out = hs.get("sess-ready").unwrap().output.lock().await;
        out.extend_from_slice(b"\x1b[>4;2m\x1b[>1u>\n? for shortcuts\r\n");
    }

    let mgr_bg = mgr.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        let hs = mgr_bg.holders.lock().await;
        if let Some(h) = hs.get("sess-ready") {
            let mut out = h.output.lock().await;
            out.extend_from_slice(b"? for shortcuts   Gemini 4 Argon\r\n");
        }
    });

    let t0 = std::time::Instant::now();
    wait_for_pty_prompt(&mgr, "sess-ready").await.unwrap();
    let elapsed = t0.elapsed().as_millis();
    assert!((1200..3000).contains(&elapsed), "Unexpected elapsed: {}ms", elapsed);
    mgr.terminate("sess-ready").await;
}

#[test]
fn test_extract_pty_turn_error_ignores_prior_and_stale_errors() {
    use super::completion::extract_pty_turn_error;

    let prior = b"> hi\r\n\xe2\x9a\xa0 Individual quota reached\r\nError ID: err-old-1\r\n";
    let mut buf = prior.to_vec();
    let offset = buf.len();
    buf.extend_from_slice(b"> 2+2?\r\n\xe2\x9a\xa0 Individual quota reached\r\nError ID: err-old-1\r\n");
    assert!(extract_pty_turn_error(&buf, offset).is_none());

    buf.extend_from_slice(b"\xe2\x9a\xa0 Individual quota reached\r\nError ID: err-new-2\r\n");
    let err = extract_pty_turn_error(&buf, offset);
    assert!(err.is_some());
    assert!(err.unwrap().contains("err-new-2"));

    buf.extend_from_slice(b"> next prompt\r\n");
    assert!(extract_pty_turn_error(&buf, offset).is_none());
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn test_pty_master_fds_have_cloexec_and_do_not_leak_to_siblings() {
    use nix::fcntl::{fcntl, FcntlArg, FdFlag};
    use super::session::SessionManager;
    use std::collections::HashSet;

    let inherited_ptmx: HashSet<String> = std::fs::read_dir("/proc/self/fd")
        .unwrap()
        .filter_map(|e| {
            let e = e.ok()?;
            let link = std::fs::read_link(e.path()).ok()?;
            link.to_string_lossy().contains("ptmx").then(|| e.file_name().to_string_lossy().into_owned())
        })
        .collect();

    let ws = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mgr = SessionManager::new();
    let env = HashMap::new();

    mgr.ensure_session("cloexec-sibling-a", &ws, "cat", &[], &env).await.unwrap();
    let master_a = mgr.holders.lock().await.get("cloexec-sibling-a").unwrap().master_fd;
    let flags_a = FdFlag::from_bits_truncate(fcntl(master_a, FcntlArg::F_GETFD).unwrap());
    assert!(flags_a.contains(FdFlag::FD_CLOEXEC), "Session A master_fd must have FD_CLOEXEC set");

    let dump_args = vec![
        "-c".to_string(),
        "for f in /proc/self/fd/*; do printf '%s -> %s\\n' \"$(basename \"$f\")\" \"$(readlink \"$f\" 2>/dev/null)\"; done; echo FD_DUMP_DONE".to_string(),
    ];
    mgr.ensure_session("cloexec-sibling-b", &ws, "sh", &dump_args, &env).await.unwrap();

    let mut dump = String::new();
    for _ in 0..30 {
        tokio::time::sleep(Duration::from_millis(40)).await;
        if let Some(h) = mgr.holders.lock().await.get("cloexec-sibling-b") {
            dump = String::from_utf8_lossy(&h.output.lock().await).to_string();
            if dump.contains("FD_DUMP_DONE") {
                break;
            }
        }
    }
    assert!(dump.contains("FD_DUMP_DONE"), "Expected FD_DUMP_DONE, got: {}", dump);
    let child_new_ptmx: Vec<&str> = dump
        .lines()
        .filter(|l| l.contains("ptmx"))
        .filter(|l| {
            let fd_num = l.split(" -> ").next().unwrap_or("").trim();
            !inherited_ptmx.contains(fd_num)
        })
        .collect();
    assert!(
        child_new_ptmx.is_empty(),
        "Sibling PTY child inherited newly spawned /dev/ptmx master FD (master_a={}): {:?}",
        master_a,
        child_new_ptmx
    );
    mgr.terminate_all().await;
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn test_pty_child_reaped_by_pdeathsig_when_parent_exits_without_drop() {
    if std::env::var("TUNER_PDEATHSIG_CHILD_HELPER").as_deref() == Ok("1") {
        let ws = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let args = vec!["-c".to_string(), "trap '' TERM HUP; sleep 60".to_string()];
        let holder = spawn_session(&ws, "sh", &args, &HashMap::new()).unwrap();
        println!("GRANDCHILD_PID={}", holder.child.id().unwrap());
        use std::io::Write;
        let _ = std::io::stdout().flush();
        std::process::exit(42);
    }

    let exe = std::env::current_exe().unwrap();
    let out = tokio::process::Command::new(exe)
        .args([
            "--exact",
            "cli::antigravity::session_tests::test_pty_child_reaped_by_pdeathsig_when_parent_exits_without_drop",
            "--nocapture",
        ])
        .env("TUNER_PDEATHSIG_CHILD_HELPER", "1")
        .output()
        .await
        .unwrap();
    assert_eq!(out.status.code(), Some(42));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let gpid: i32 = stdout
        .lines()
        .find_map(|l| l.strip_prefix("GRANDCHILD_PID="))
        .expect("Missing GRANDCHILD_PID")
        .trim()
        .parse()
        .unwrap();

    let nix_pid = nix::unistd::Pid::from_raw(gpid);
    let mut dead = false;
    for _ in 0..25 {
        if nix::sys::signal::kill(nix_pid, None).is_err() {
            dead = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(dead, "Orphaned PTY grandchild {} survived parent exit(42)", gpid);
}
