//! # Antigravity CLI Interrupt Tests
//!
//! Unit tests verifying ESC soft interruption, session preservation,
//! and immediate breakout from log completion polling loops.

#[cfg(test)]
mod tests {
    use crate::cli::antigravity::session::SessionManager;
    use crate::cli::antigravity::polling::wait_for_log_completion;
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    fn setup_env(chat_id: i64, topic_id: Option<i64>) -> HashMap<String, String> {
        let mut env = HashMap::new();
        env.insert("TUNER_CHAT_ID".to_string(), chat_id.to_string());
        if let Some(tid) = topic_id {
            env.insert("TUNER_TOPIC_ID".to_string(), tid.to_string());
        }
        env
    }

    #[tokio::test]
    async fn test_session_interrupt_sends_esc_and_marks_interrupted() {
        let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let env = setup_env(1001, Some(2002));
        let manager = SessionManager::new();

        // 1. Ensure active session
        let res = manager.ensure_session("sess-int-1", &workspace, "cat", &[], &env).await;
        assert!(res.is_ok());
        manager.set_running("sess-int-1", true).await;

        // 2. Interrupt active session in the matching topic
        let interrupted = manager.interrupt(1001, Some(2002)).await;
        assert!(interrupted, "Interrupt should match running session in topic 2002");
        assert!(manager.is_interrupted("sess-int-1").await, "Session should be marked interrupted");

        // 3. Process must STILL be alive (not SIGKILLed)
        assert!(manager.is_active("sess-int-1").await, "Session process should remain alive");

        // 4. Interrupting again without running/ask state returns false
        manager.clear_interrupted("sess-int-1").await;
        manager.set_running("sess-int-1", false).await;
        let interrupted_again = manager.interrupt(1001, Some(2002)).await;
        assert!(!interrupted_again, "Inactive session should not be interrupted");

        manager.terminate_all().await;
    }

    #[tokio::test]
    async fn test_wait_for_log_completion_exits_early_on_interrupt() {
        let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let env = setup_env(5555, Some(7777));
        let manager = std::sync::Arc::new(SessionManager::new());

        let res = manager.ensure_session("sess-poll-int", &workspace, "cat", &[], &env).await;
        assert!(res.is_ok());
        manager.set_running("sess-poll-int", true).await;

        let mgr_clone = manager.clone();
        let start = Instant::now();
        let poll_task = tokio::spawn(async move {
            wait_for_log_completion(&mgr_clone, "sess-poll-int", None, 0).await
        });

        // Trigger interrupt after 25ms
        tokio::time::sleep(Duration::from_millis(25)).await;
        let interrupted = manager.interrupt(5555, Some(7777)).await;
        assert!(interrupted);

        let poll_res = poll_task.await.expect("Task should join cleanly");
        let elapsed = start.elapsed();

        // Must break out immediately (well below 500ms, not waiting for 300s timeout)
        assert!(elapsed < Duration::from_millis(500), "Poll loop took too long: {:?}", elapsed);
        assert_eq!(poll_res, Err("Interrupted by user (/stop)".to_string()));

        // Check that interrupted flag was cleared and session remains alive
        assert!(!manager.is_interrupted("sess-poll-int").await);
        assert!(manager.is_active("sess-poll-int").await);

        manager.terminate_all().await;
    }

    #[tokio::test]
    async fn test_interrupt_nonexistent_topic_returns_false() {
        let manager = SessionManager::new();
        let interrupted = manager.interrupt(99999, Some(11111)).await;
        assert!(!interrupted);
    }
}
