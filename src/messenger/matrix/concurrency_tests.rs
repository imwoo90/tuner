#[cfg(test)]
mod tests {
    use crate::messenger::matrix::message_queue::MatrixMessageQueue;
    use crate::bus::lock_pool::LockPool;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::mpsc;

    // Helper function simulating Matrix message task processing with lock pool serialization
    async fn sim_process_message_task(
        queue: Arc<MatrixMessageQueue>,
        lock_pool: Arc<LockPool>,
        chat_id: i64,
        duration: Duration,
        step_tx: mpsc::Sender<(i64, &'static str)>,
    ) {
        let step_tx = step_tx.clone();
        let task = tokio::spawn(async move {
            let lock = lock_pool.get_chat(chat_id);
            let _guard = lock.lock().await;

            let _ = step_tx.send((chat_id, "start")).await;
            tokio::time::sleep(duration).await;
            let _ = step_tx.send((chat_id, "end")).await;
        });
        queue.track(chat_id, task);
    }

    #[tokio::test]
    async fn test_same_room_sequential_processing() {
        let queue = Arc::new(MatrixMessageQueue::new());
        let lock_pool = Arc::new(LockPool::new_default());
        let (tx, mut rx) = mpsc::channel(10);

        // Queue two messages for the same room (chat_id = 1)
        // Message 1: 150ms
        // Message 2: 50ms
        sim_process_message_task(queue.clone(), lock_pool.clone(), 1, Duration::from_millis(150), tx.clone()).await;
        sim_process_message_task(queue.clone(), lock_pool.clone(), 1, Duration::from_millis(50), tx.clone()).await;

        let mut events = Vec::new();
        // Wait for all 4 events: start1, end1, start2, end2
        let timeout = tokio::time::timeout(Duration::from_millis(500), async {
            while let Some(ev) = rx.recv().await {
                events.push(ev);
                if events.len() == 4 {
                    break;
                }
            }
        }).await;

        assert!(timeout.is_ok(), "Test timed out before all events were processed");
        
        // Assert sequential execution:
        // Event 0: (1, "start")
        // Event 1: (1, "end")
        // Event 2: (1, "start")
        // Event 3: (1, "end")
        assert_eq!(events, vec![
            (1, "start"),
            (1, "end"),
            (1, "start"),
            (1, "end")
        ]);
        
        assert_eq!(queue.pending_count(1), 0);
    }

    #[tokio::test]
    async fn test_different_rooms_parallel_processing() {
        let queue = Arc::new(MatrixMessageQueue::new());
        let lock_pool = Arc::new(LockPool::new_default());
        let (tx, mut rx) = mpsc::channel(10);

        // Queue message 1 for room 1 (chat_id = 1) with 200ms duration
        // Queue message 2 for room 2 (chat_id = 2) with 50ms duration
        sim_process_message_task(queue.clone(), lock_pool.clone(), 1, Duration::from_millis(200), tx.clone()).await;
        // Small delay to ensure order of execution begins sequentially, but runs concurrently
        tokio::time::sleep(Duration::from_millis(10)).await;
        sim_process_message_task(queue.clone(), lock_pool.clone(), 2, Duration::from_millis(50), tx.clone()).await;

        let mut events = Vec::new();
        let timeout = tokio::time::timeout(Duration::from_millis(500), async {
            while let Some(ev) = rx.recv().await {
                events.push(ev);
                if events.len() == 4 {
                    break;
                }
            }
        }).await;

        assert!(timeout.is_ok(), "Test timed out before all events were processed");

        // Assert parallel execution:
        // Message 1 starts first.
        // Message 2 starts immediately after (parallel, doesn't wait for message 1 to finish).
        // Message 2 finishes first (since it's only 50ms vs Message 1's 200ms).
        // Message 1 finishes last.
        assert_eq!(events, vec![
            (1, "start"),
            (2, "start"),
            (2, "end"),
            (1, "end")
        ]);

        assert_eq!(queue.pending_count(1), 0);
        assert_eq!(queue.pending_count(2), 0);
    }

    #[tokio::test]
    async fn test_stop_command_immediate_cancellation() {
        let queue = Arc::new(MatrixMessageQueue::new());
        let lock_pool = Arc::new(LockPool::new_default());
        let (tx, mut rx) = mpsc::channel(10);

        // Queue a long running message in room 1 (chat_id = 1)
        sim_process_message_task(queue.clone(), lock_pool.clone(), 1, Duration::from_millis(500), tx.clone()).await;

        // Wait for it to start
        let start_event = tokio::time::timeout(Duration::from_millis(100), rx.recv()).await;
        assert_eq!(start_event.unwrap().unwrap(), (1, "start"));

        // Verify it is pending/running
        assert_eq!(queue.pending_count(1), 1);
        assert!(lock_pool.is_locked(1));

        // Simulate `/stop` command by calling queue.drain(1)
        let cancelled = queue.drain(1);
        assert_eq!(cancelled, 1);

        // Verify task was immediately removed from pending
        assert_eq!(queue.pending_count(1), 0);

        // Wait to make sure the task does NOT produce "end" event
        tokio::time::sleep(Duration::from_millis(150)).await;
        
        // Assert no more events are in the channel (the task was aborted)
        assert!(rx.try_recv().is_err());

        // Verify the lock is now released
        assert!(!lock_pool.is_locked(1));

        // Verify a new task can acquire the lock immediately and run
        sim_process_message_task(queue.clone(), lock_pool.clone(), 1, Duration::from_millis(50), tx.clone()).await;
        
        let start_event_new = tokio::time::timeout(Duration::from_millis(100), rx.recv()).await;
        assert_eq!(start_event_new.unwrap().unwrap(), (1, "start"));
        
        let end_event_new = tokio::time::timeout(Duration::from_millis(100), rx.recv()).await;
        assert_eq!(end_event_new.unwrap().unwrap(), (1, "end"));
    }
}
