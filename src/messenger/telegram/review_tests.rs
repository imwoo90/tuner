#![allow(clippy::await_holding_lock)]
#[cfg(test)]
mod tests {
    use super::super::review::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_create_review_session_and_render_html() {
        let dir = tempdir().unwrap();
        let file1 = dir.path().join("main.c");
        let file2 = dir.path().join("config.toml");
        std::fs::write(&file1, "int main(void) {\n    /* multi-line\n       comment */\n    return 0;\n}").unwrap();
        std::fs::write(&file2, "[app]\nname = \"tuner\"").unwrap();

        let mgr = ReviewManager::new();
        let (token, count) = mgr.create_session(&[file1.clone(), file2.clone()]).await.unwrap();

        assert_eq!(count, 2);
        assert!(!token.is_empty());

        let files = mgr.get_files(&token).await.unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].filename, "main.c");
        assert_eq!(files[0].language, "c");
        assert_eq!(files[1].filename, "config.toml");
        assert_eq!(files[1].language, "toml");

        let html = mgr.get_html(&token).await.unwrap();
        assert!(html.contains("Tuner File Review"));
        assert!(html.contains("main.c"));
        assert!(html.contains("config.toml"));
        assert!(html.contains("splitHighlightedLines"));
        assert!(html.contains("code-lines"));
    }

    #[tokio::test]
    async fn test_video_and_binary_session() {
        let dir = tempdir().unwrap();
        let video_file = dir.path().join("sample.mp4");
        let fake_video_data = vec![0u8; 1024];
        std::fs::write(&video_file, &fake_video_data).unwrap();

        let mgr = ReviewManager::new();
        let (token, count) = mgr.create_session(std::slice::from_ref(&video_file)).await.unwrap();
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
        if let Ok(res) = client.get(format!("http://127.0.0.1:{}/health", port)).send().await
            && res.status() == 200 {
                return;
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

    #[tokio::test]
    async fn test_oversized_html_caps_inline_content_and_preserves_size() {
        let dir = tempdir().unwrap();
        let big_html = dir.path().join("viewer_3d.html");
        let payload = format!("<!DOCTYPE html><html><script>const DATA = \"{}\";</script></html>", "A".repeat(400_000));
        let expected_size = payload.len();
        std::fs::write(&big_html, &payload).unwrap();

        let mgr = ReviewManager::new();
        let (token, count) = mgr.create_session(std::slice::from_ref(&big_html)).await.unwrap();
        assert_eq!(count, 1);

        let files = mgr.get_files(&token).await.unwrap();
        assert_eq!(files[0].filename, "viewer_3d.html");
        assert_eq!(files[0].language, "html");
        assert_eq!(files[0].size_bytes, expected_size);
        assert!(files[0].content.contains("File exceeds 256KB"));
        assert!(files[0].content.len() < 256);

        let html = mgr.get_html(&token).await.unwrap();
        assert!(html.len() < 80_000, "Rendered review HTML must stay lightweight (<80KB), got {}", html.len());
        assert!(html.contains("showHtmlView"));
    }

    #[tokio::test]
    async fn test_zip_archive_inspection_in_review_session() {
        let dir = tempdir().unwrap();
        let part_file = dir.path().join("front_shell.step");
        std::fs::write(&part_file, "ISO-10303-21; STEP CONTENT").unwrap();

        let sample_rf = ReviewFile {
            filename: "front_shell.step".to_string(),
            path: part_file.to_string_lossy().to_string(),
            content: String::new(),
            size_bytes: 26,
            language: "plaintext".to_string(),
            is_binary: true,
        };
        let zip_bytes = super::super::attachments_dl::create_zip_archive(&[sample_rf]).unwrap();
        let zip_path = dir.path().join("release_bundle.zip");
        std::fs::write(&zip_path, &zip_bytes).unwrap();

        let mgr = ReviewManager::new();
        let (token, count) = mgr.create_session(std::slice::from_ref(&zip_path)).await.unwrap();
        assert_eq!(count, 1);

        let files = mgr.get_files(&token).await.unwrap();
        assert_eq!(files[0].filename, "release_bundle.zip");
        assert_eq!(files[0].language, "zip");
        assert!(files[0].is_binary);
        assert_eq!(files[0].size_bytes, zip_bytes.len());
        assert!(files[0].content.contains("front_shell.step"));
    }
}
