// Proposed: src/background_tests_extra.rs

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
    async fn test_timeout_status() {
        let paths = make_paths();
        // Set short timeout to trigger timeout error status
        let observer = BackgroundObserver::new(paths, Duration::from_millis(50));
        let config = CliConfig::default();

        let notify = Arc::new(Notify::new());
        let mock_provider = Arc::new(MockProvider {
            notify: Some(notify.clone()),
            response: Ok(CliResponse {
                session_id: None,
                result: "slow result".to_string(),
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
            .submit(mock_provider, make_submit(123, "slow task", 1), config)
            .await
            .unwrap();

        let received = tokio::time::timeout(Duration::from_millis(150), rx.recv())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(received.status, BackgroundResultStatus::ErrorTimeout);

        // Cleanup blocker
        notify.notify_one();
    }

    #[tokio::test]
    async fn test_cancel_all() {
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
            .submit(
                mock_provider.clone(),
                make_submit(123, "task1", 1),
                config.clone(),
            )
            .await
            .unwrap();
        observer
            .submit(
                mock_provider.clone(),
                make_submit(123, "task2", 2),
                config,
            )
            .await
            .unwrap();

        tokio::time::sleep(Duration::from_millis(10)).await;

        let cancelled = observer.cancel_all(123).await;
        assert_eq!(cancelled, 2);
    }

    #[tokio::test]
    async fn test_cancel_delivers_aborted() {
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
            .submit(mock_provider, make_submit(123, "cancellable", 1), config)
            .await
            .unwrap();

        tokio::time::sleep(Duration::from_millis(10)).await;

        observer.cancel_all(123).await;

        let received = tokio::time::timeout(Duration::from_millis(100), rx.recv())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(received.status, BackgroundResultStatus::Aborted);
    }

    #[tokio::test]
    async fn test_shutdown_cancels_all() {
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
            .submit(
                mock_provider.clone(),
                make_submit(123, "t1", 1),
                config.clone(),
            )
            .await
            .unwrap();
        observer
            .submit(
                mock_provider.clone(),
                make_submit(456, "t2", 2),
                config,
            )
            .await
            .unwrap();

        tokio::time::sleep(Duration::from_millis(10)).await;

        observer.shutdown().await;

        assert_eq!(observer.active_tasks(None).await.len(), 0);
    }

    #[tokio::test]
    async fn test_task_removed_after_completion() {
        let paths = make_paths();
        let observer = BackgroundObserver::new(paths, Duration::from_secs(300));
        let config = CliConfig::default();

        let mock_provider = Arc::new(MockProvider {
            notify: None,
            response: Ok(CliResponse {
                session_id: None,
                result: "ok".to_string(),
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
            .submit(mock_provider, make_submit(123, "quick", 1), config)
            .await
            .unwrap();

        let _ = tokio::time::timeout(Duration::from_millis(100), rx.recv())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(observer.active_tasks(Some(123)).await.len(), 0);
    }
}
