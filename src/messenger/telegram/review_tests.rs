#[cfg(test)]
mod tests {
    use super::super::review::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_create_review_session_and_render_html() {
        let dir = tempdir().unwrap();
        let file1 = dir.path().join("main.rs");
        let file2 = dir.path().join("config.toml");
        std::fs::write(&file1, "fn main() { println!(\"hello\"); }").unwrap();
        std::fs::write(&file2, "[app]\nname = \"tuner\"").unwrap();

        let mgr = ReviewManager::new();
        let (token, count) = mgr.create_session(&[file1.clone(), file2.clone()]).await.unwrap();

        assert_eq!(count, 2);
        assert!(!token.is_empty());

        let files = mgr.get_files(&token).await.unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].filename, "main.rs");
        assert_eq!(files[0].language, "rust");
        assert_eq!(files[1].filename, "config.toml");
        assert_eq!(files[1].language, "toml");

        let html = mgr.get_html(&token).await.unwrap();
        assert!(html.contains("Tuner File Review"));
        assert!(html.contains("main.rs"));
        assert!(html.contains("config.toml"));
    }

    #[tokio::test]
    async fn test_video_and_binary_session() {
        let dir = tempdir().unwrap();
        let video_file = dir.path().join("sample.mp4");
        let fake_video_data = vec![0u8; 1024];
        std::fs::write(&video_file, &fake_video_data).unwrap();

        let mgr = ReviewManager::new();
        let (token, count) = mgr.create_session(&[video_file.clone()]).await.unwrap();
        assert_eq!(count, 1);

        let files = mgr.get_files(&token).await.unwrap();
        assert_eq!(files[0].filename, "sample.mp4");
        assert_eq!(files[0].language, "video");
        assert!(files[0].is_binary);
        assert_eq!(files[0].size_bytes, 1024);
        assert!(files[0].content.is_empty());
    }

async fn wait_for_server_healthy(port: u16, client: &reqwest::Client) {
    for _ in 0..20 {
        if let Ok(res) = client.get(format!("http://127.0.0.1:{}/health", port)).send().await {
            if res.status() == 200 {
                return;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("Review server health endpoint failed to respond");
}

    #[tokio::test]
    async fn test_review_server_and_media_endpoints() {
        let _lock = super::super::TEST_ENV_MUTEX.lock().unwrap();
        let dir = tempdir().unwrap();
        let py_file = dir.path().join("test.py");
        std::fs::write(&py_file, "print('hello from test')").unwrap();
        let video_file = dir.path().join("video.mp4");
        std::fs::write(&video_file, b"fake video byte stream content").unwrap();

        let mgr = global_review_manager();
        let (token, _) = mgr.create_session(&[py_file, video_file]).await.unwrap();
        let port = mgr.ensure_server_running().await;

        let client = reqwest::Client::new();
        wait_for_server_healthy(port, &client).await;

        // Review page
        let review_res = client.get(format!("http://127.0.0.1:{}/review/{}", port, token)).send().await.unwrap();
        assert_eq!(review_res.status(), 200);
        let body = review_res.text().await.unwrap();
        assert!(body.contains("test.py"));

        // Media streaming endpoint
        let media_res = client.get(format!("http://127.0.0.1:{}/review/{}/media/video.mp4", port, token)).send().await.unwrap();
        assert_eq!(media_res.status(), 200);
        assert_eq!(media_res.bytes().await.unwrap(), &b"fake video byte stream content"[..]);

        // Download endpoint
        let dl_res = client.get(format!("http://127.0.0.1:{}/review/{}/download/video.mp4", port, token)).send().await.unwrap();
        assert_eq!(dl_res.status(), 200);
        let disp = dl_res.headers().get("content-disposition").unwrap().to_str().unwrap();
        assert!(disp.contains("attachment; filename=\"video.mp4\""));

        // Invalid token 404
        let not_found = client.get(format!("http://127.0.0.1:{}/review/invalid-token-12345", port)).send().await.unwrap();
        assert_eq!(not_found.status(), 404);
    }

    #[tokio::test]
    async fn test_review_store_persistence_and_missing_files() {
        let dir = tempdir().unwrap();
        let file1 = dir.path().join("first.rs");
        let file2 = dir.path().join("second.rs");
        std::fs::write(&file1, "fn first() {}").unwrap();
        std::fs::write(&file2, "fn second() {}").unwrap();

        let storage_file = dir.path().join("review_sessions.json");
        let mgr = ReviewManager::new();
        mgr.init_storage(storage_file.clone()).await;

        let (session_id, count) = mgr.register_session(&[file1.clone(), file2.clone()]).await.unwrap();
        assert_eq!(count, 2);

        // Verify record persisted to disk
        let record = mgr.get_session_record(&session_id).await.unwrap();
        assert_eq!(record.file_paths.len(), 2);

        // Simulate deleting one file from disk
        std::fs::remove_file(&file1).unwrap();
        let (valid_paths, total) = super::super::review_store::check_record_files(&record);
        assert_eq!(total, 2);
        assert_eq!(valid_paths.len(), 1);
        assert_eq!(valid_paths[0], file2);

        // Simulate deleting all files from disk
        std::fs::remove_file(&file2).unwrap();
        let (valid_paths_empty, _) = super::super::review_store::check_record_files(&record);
        assert!(valid_paths_empty.is_empty());
    }
}
