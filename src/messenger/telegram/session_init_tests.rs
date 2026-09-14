//! # Session Initializer Tests
//!
//! Validates that initialize_session_if_needed properly detects expired/pruned
//! Antigravity sessions and re-initializes clean sessions without hanging.

#[cfg(test)]
mod tests {
    use crate::config::CliConfig;
    use crate::cli::antigravity::AntigravityCli;
    use crate::session::key::SessionKey;
    use crate::session::manager::SessionManager;
    use std::sync::Arc;
    use teloxide::Bot;
    use teloxide::types::Message;

    fn make_msg(json: &str) -> Message {
        serde_json::from_str(json).unwrap()
    }

    #[tokio::test]
    async fn test_initialize_session_resets_expired_session() {
        let temp = tempfile::NamedTempFile::new().unwrap();
        let mgr = Arc::new(SessionManager::new(temp.path().to_path_buf(), 30, 4, false, "UTC".to_string(), None));
        let cfg = Arc::new(CliConfig {
            provider: "antigravity".to_string(),
            allowed_user_ids: vec![100],
            ..Default::default()
        });
        let cli = Arc::new(AntigravityCli::new((*cfg).clone()));
        let bot = Bot::new("123:abc");
        let key = SessionKey::telegram(123, None);
        let (mut sess, _) = mgr.resolve_session(&key, &cfg.provider, "opus").await.unwrap();

        sess.set_session_id("antigravity", "expired-uuid-12345");
        mgr.update_session(&sess, 0.0, 0).await.unwrap();

        let msg = make_msg(r#"{"message_id":13,"date":1,"chat":{"id":123,"type":"private"},"from":{"id":100,"is_bot":false,"first_name":"I"},"text":"hello"}"#);

        let active_id = crate::messenger::telegram::session_init::initialize_session_if_needed(
            &bot,
            &msg,
            &mgr,
            &mut sess,
            &cli,
            &cfg,
        ).await.unwrap();

        assert_eq!(active_id, "mock-session-123");
        assert_eq!(sess.get_session_id("antigravity"), "mock-session-123");
    }

    #[tokio::test]
    async fn test_initialize_session_preserves_valid_mock_session() {
        let temp = tempfile::NamedTempFile::new().unwrap();
        let mgr = Arc::new(SessionManager::new(temp.path().to_path_buf(), 30, 4, false, "UTC".to_string(), None));
        let cfg = Arc::new(CliConfig {
            provider: "antigravity".to_string(),
            ..Default::default()
        });
        let cli = Arc::new(AntigravityCli::new((*cfg).clone()));
        let bot = Bot::new("123:abc");
        let key = SessionKey::telegram(123, None);
        let (mut sess, _) = mgr.resolve_session(&key, &cfg.provider, "opus").await.unwrap();

        sess.set_session_id("antigravity", "mock-session-valid");
        mgr.update_session(&sess, 0.0, 0).await.unwrap();

        let msg = make_msg(r#"{"message_id":14,"date":1,"chat":{"id":123,"type":"private"},"from":{"id":100,"is_bot":false,"first_name":"I"},"text":"hello"}"#);

        let active_id = crate::messenger::telegram::session_init::initialize_session_if_needed(
            &bot,
            &msg,
            &mgr,
            &mut sess,
            &cli,
            &cfg,
        ).await.unwrap();

        assert_eq!(active_id, "mock-session-valid");
    }
}
