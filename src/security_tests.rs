//! # Security Tests for paths and content modules
//!
//! Replicates Python security test suite in compact, idiomatic Rust.

#[cfg(test)]
mod tests {
    use crate::security::paths::{validate_file_path, is_path_safe, PathValidationError};

    // =========================================================================
    // Path Safety Tests
    // =========================================================================

    #[test]
    fn test_valid_path_inside_root() {
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("test.txt");
        std::fs::write(&f, "ok").unwrap();

        let allowed_roots = vec![tmp.path().to_path_buf()];
        let result = validate_file_path(&f, &allowed_roots).unwrap();
        assert_eq!(result, f.canonicalize().unwrap());
    }

    #[test]
    fn test_path_outside_all_roots() {
        let tmp = tempfile::tempdir().unwrap();
        let allowed_roots = vec![tmp.path().to_path_buf()];
        let res = validate_file_path("/etc/passwd", &allowed_roots);
        assert!(matches!(res, Err(PathValidationError::OutsideAllowedRoots(_, _))));
    }

    #[test]
    fn test_null_byte_in_path() {
        let tmp = tempfile::tempdir().unwrap();
        let allowed_roots = vec![tmp.path().to_path_buf()];
        let res = validate_file_path("/tmp/evil\x00file", &allowed_roots);
        if let Err(PathValidationError::NullByte(raw)) = res {
            assert!(raw.contains("evil"));
        } else {
            panic!("Expected NullByte, got {:?}", res);
        }
    }

    #[test]
    fn test_control_characters_in_path() {
        let tmp = tempfile::tempdir().unwrap();
        let allowed_roots = vec![tmp.path().to_path_buf()];
        let res = validate_file_path("/tmp/evil\x01file", &allowed_roots);
        if let Err(PathValidationError::ControlCharacters(raw)) = res {
            assert!(raw.contains("evil"));
        } else {
            panic!("Expected ControlCharacters, got {:?}", res);
        }
    }

    #[test]
    fn test_multiple_allowed_roots() {
        let tmp = tempfile::tempdir().unwrap();
        let root_a = tmp.path().join("a");
        let root_b = tmp.path().join("b");
        std::fs::create_dir(&root_a).unwrap();
        std::fs::create_dir(&root_b).unwrap();
        let f = root_b.join("file.txt");
        std::fs::write(&f, "ok").unwrap();

        let allowed_roots = vec![root_a, root_b];
        let result = validate_file_path(&f, &allowed_roots).unwrap();
        assert_eq!(result, f.canonicalize().unwrap());
    }

    #[test]
    fn test_is_path_safe_returns_true() {
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("safe.txt");
        std::fs::write(&f, "ok").unwrap();
        assert!(is_path_safe(&f, &[tmp.path().to_path_buf()]));
    }

    #[test]
    fn test_is_path_safe_returns_false() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(!is_path_safe("/etc/shadow", &[tmp.path().to_path_buf()]));
    }

    #[test]
    fn test_is_path_safe_no_exception_on_bad_path() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(!is_path_safe("/nonexistent/\x00evil", &[tmp.path().to_path_buf()]));
    }

    #[test]
    fn test_symlink_traversal() {
        let tmp = tempfile::tempdir().unwrap();
        let allowed = tmp.path().join("allowed");
        let outside = tmp.path().join("outside");
        std::fs::create_dir(&allowed).unwrap();
        std::fs::create_dir(&outside).unwrap();
        let secret = outside.join("secret.txt");
        std::fs::write(&secret, "sensitive").unwrap();
        let link = allowed.join("link.txt");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&secret, &link).unwrap();

        let res = validate_file_path(&link, &[allowed]);
        assert!(matches!(res, Err(PathValidationError::OutsideAllowedRoots(_, _))));
    }

    #[test]
    fn test_dotdot_traversal_blocked() {
        let tmp = tempfile::tempdir().unwrap();
        let allowed = tmp.path().join("allowed");
        let outside = tmp.path().join("outside");
        std::fs::create_dir(&allowed).unwrap();
        std::fs::create_dir(&outside).unwrap();
        let secret = outside.join("secret.txt");
        std::fs::write(&secret, "secret").unwrap();

        let sneaky = allowed.join("..").join("outside").join("secret.txt");
        assert!(validate_file_path(&sneaky, &[allowed]).is_err());
    }

    #[test]
    fn test_relative_path_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let allowed = tmp.path().join("allowed");
        std::fs::create_dir(&allowed).unwrap();
        assert!(validate_file_path("./sneaky.txt", &[allowed]).is_err());
    }

    #[test]
    fn test_vault_store_roundtrip_and_masking() {
        use crate::security::vault::{mask_username, normalize_site_key, VaultEntry, VaultStore};
        use crate::security::vault_cli::redact_secrets;

        assert_eq!(normalize_site_key("  JLCPCB Order / Main "), "jlcpcb-order-main");
        assert_eq!(normalize_site_key("네이버 사내 포털"), "네이버-사내-포털");
        assert_eq!(mask_username("imwoo90@gmail.com"), "im***@gmail.com");
        assert_eq!(mask_username("admin"), "adm***");

        let tmp = tempfile::tempdir().unwrap();
        let store = VaultStore::new(tmp.path().join("vault"));
        assert!(store.load_entries().unwrap().is_empty());

        let entry = VaultEntry {
            site_key: "JLCPCB".to_string(),
            title: "JLCPCB PCB/PCBA".to_string(),
            url: "https://jlcpcb.com".to_string(),
            username: "imwoo90@example.com".to_string(),
            password: "SuperSecretPassword!2026".to_string(),
            updated_at_secs: 0,
        };
        let saved = store.upsert_entry(entry).unwrap();
        assert_eq!(saved.site_key, "jlcpcb");

        // Verify disk payload is encrypted (no plaintext password in vault.enc)
        let raw_enc = std::fs::read_to_string(tmp.path().join("vault").join("vault.enc")).unwrap();
        assert!(!raw_enc.contains("SuperSecretPassword!2026"));

        let loaded = store.get_entry("jlcpcb").unwrap().unwrap();
        assert_eq!(loaded.password, "SuperSecretPassword!2026");

        let masked = store.list_masked().unwrap();
        assert_eq!(masked.len(), 1);
        assert_eq!(masked[0].masked_username, "im***@example.com");
        assert!(masked[0].has_password);

        let scrubbed = redact_secrets(
            "Login failed with pass=SuperSecretPassword!2026",
            &[&loaded.password],
        );
        assert!(!scrubbed.contains("SuperSecretPassword!2026"));
        assert!(scrubbed.contains("***REDACTED***"));

        assert!(store.delete_entry("jlcpcb").unwrap());
        assert!(store.load_entries().unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_vault_web_endpoints_and_lock() {
        use axum::body::Body;
        use axum::http::{Request, StatusCode};
        use tower::ServiceExt;

        let tmp = tempfile::tempdir().unwrap();
        let vault_dir = tmp.path().join("vault");
        let mgr = crate::telegram::vault::global_vault_manager();
        let token = mgr.create_session_for("default".to_string(), vault_dir).await;

        let app = crate::telegram::review_server::build_review_router();

        // 1. GET /vault/:token renders HTML
        let res = app
            .clone()
            .oneshot(Request::builder().uri(format!("/vault/{}", token)).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        // 2. POST /vault/:token/api/entries creates a site entry
        let body_json = serde_json::json!({
            "site_key": "jlcpcb",
            "title": "JLCPCB",
            "url": "https://jlcpcb.com",
            "username": "imwoo90@example.com",
            "password": "MyVaultPass#99"
        });
        let res_post = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/vault/{}/api/entries", token))
                    .header("content-type", "application/json")
                    .body(Body::from(body_json.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res_post.status(), StatusCode::OK);

        // 3. POST /vault/:token/api/lock invalidates the session
        let res_lock = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/vault/{}/api/lock", token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res_lock.status(), StatusCode::OK);

        // 4. Subsequent GET /vault/:token returns 404
        let res_after = app
            .oneshot(Request::builder().uri(format!("/vault/{}", token)).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res_after.status(), StatusCode::NOT_FOUND);
    }
}
