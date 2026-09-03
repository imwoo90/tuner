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
    async fn test_review_server_endpoints() {
        let dir = tempdir().unwrap();
        let sample = dir.path().join("test.py");
        std::fs::write(&sample, "print('hello from test')").unwrap();

        let mgr = global_review_manager();
        let (token, _) = mgr.create_session(&[sample]).await.unwrap();
        let port = mgr.ensure_server_running().await;

        let client = reqwest::Client::new();
        let health_res = client.get(format!("http://127.0.0.1:{}/health", port)).send().await;
        assert!(health_res.is_ok());
        assert_eq!(health_res.unwrap().status(), 200);

        let review_res = client.get(format!("http://127.0.0.1:{}/review/{}", port, token)).send().await;
        assert!(review_res.is_ok());
        let resp = review_res.unwrap();
        assert_eq!(resp.status(), 200);
        let body = resp.text().await.unwrap();
        assert!(body.contains("test.py"));

        let not_found = client.get(format!("http://127.0.0.1:{}/review/invalid-token-12345", port)).send().await;
        assert_eq!(not_found.unwrap().status(), 404);
    }
}
