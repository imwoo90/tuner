// Proposed: src/background_tests.rs

#[cfg(test)]
mod tests {
    use crate::background::observer::*;
    use crate::background::test_utils::*;
    use crate::cli::CliResponse;
    use crate::config::CliConfig;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::{mpsc, Notify};

    #[tokio::test]
    async fn test_returns_task_id() {
        let paths = make_paths();
        let observer = BackgroundObserver::new(paths, Duration::from_secs(300));
        let config = CliConfig::default();

        let mock_provider = Arc::new(MockProvider {
            notify: None,
            response: Err("CLI not found".to_string()),
        });

        // Set dummy handler
        observer.set_result_handler(|_| {}).await;

        let task_id = observer
            .submit(mock_provider, make_submit(123, "test prompt", 1), config)
            .await
            .unwrap();

        assert_eq!(task_id.len(), 8);
    }

    #[tokio::test]
    async fn test_task_appears_in_active() {
        let paths = make_paths();
        let observer = BackgroundObserver::new(paths, Duration::from_secs(300));
        let config = CliConfig::default();

        let notify = Arc::new(Notify::new());
        let mock_provider = Arc::new(MockProvider {
            notify: Some(notify.clone()),
            response: Ok(CliResponse {
                session_id: None,
                result: "success".to_string(),
                is_error: false,
                returncode: Some(0),
                stderr: String::new(),
            }),
        });

        observer.set_result_handler(|_| {}).await;

        observer
            .submit(mock_provider, make_submit(123, "test", 1), config)
            .await
            .unwrap();

        // Yield to allow spawn to register active task
        tokio::time::sleep(Duration::from_millis(10)).await;

        assert_eq!(observer.active_tasks(Some(123)).await.len(), 1);
        assert_eq!(observer.active_tasks(Some(999)).await.len(), 0);

        // Resume and wait
        notify.notify_one();
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    #[tokio::test]
    async fn test_max_tasks_limit() {
        let paths = make_paths();
        let observer = BackgroundObserver::new(paths, Duration::from_secs(300));
        let config = CliConfig::default();

        let notify = Arc::new(Notify::new());
        let mock_provider = Arc::new(MockProvider {
            notify: Some(notify.clone()),
            response: Ok(CliResponse {
                session_id: None,
                result: "success".to_string(),
                is_error: false,
                returncode: Some(0),
                stderr: String::new(),
            }),
        });

        observer.set_result_handler(|_| {}).await;

        for i in 0..MAX_TASKS_PER_CHAT {
            observer
                .submit(
                    mock_provider.clone(),
                    make_submit(123, "task", i as i64),
                    config.clone(),
                )
                .await
                .unwrap();
        }

        let res = observer
            .submit(
                mock_provider.clone(),
                make_submit(123, "one more", 999),
                config,
            )
            .await;

        assert!(res.is_err());
        assert!(res.unwrap_err().contains("Too many"));

        // Release tasks
        for _ in 0..MAX_TASKS_PER_CHAT {
            notify.notify_one();
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    #[tokio::test]
    async fn test_success_delivers_result() {
        let paths = make_paths();
        let observer = BackgroundObserver::new(paths, Duration::from_secs(300));
        let config = CliConfig::default();

        let mock_provider = Arc::new(MockProvider {
            notify: None,
            response: Ok(CliResponse {
                session_id: None,
                result: "Hello world".to_string(),
                is_error: false,
                returncode: Some(0),
                stderr: String::new(),
            }),
        });

        let (tx, mut rx) = mpsc::channel(1);
        observer
            .set_result_handler(move |res| {
                let tx = tx.clone();
                tokio::spawn(async move {
                    let _ = tx.send(res).await;
                });
            })
            .await;

        observer
            .submit(mock_provider, make_submit(123, "say hello", 42), config)
            .await
            .unwrap();

        let received = tokio::time::timeout(Duration::from_millis(100), rx.recv())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(received.status, BackgroundResultStatus::Success);
        assert_eq!(received.result_text, "Hello world");
        assert_eq!(received.chat_id, 123);
        assert_eq!(received.message_id, 42);
        assert_eq!(received.prompt_preview, "say hello");
    }

    #[tokio::test]
    async fn test_cli_not_found() {
        let paths = make_paths();
        let observer = BackgroundObserver::new(paths, Duration::from_secs(300));
        let config = CliConfig::default();

        let mock_provider = Arc::new(MockProvider {
            notify: None,
            response: Err("CLI not found".to_string()),
        });

        let (tx, mut rx) = mpsc::channel(1);
        observer
            .set_result_handler(move |res| {
                let tx = tx.clone();
                tokio::spawn(async move {
                    let _ = tx.send(res).await;
                });
            })
            .await;

        observer
            .submit(mock_provider, make_submit(123, "test", 1), config)
            .await
            .unwrap();

        let received = tokio::time::timeout(Duration::from_millis(100), rx.recv())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(received.status, BackgroundResultStatus::ErrorCliNotFound);
    }
}
