//! # Extra Telegram Reply Tests
//!
//! Supplemental tests for resolve_session_model and parse_model_directive.

#[cfg(test)]
mod tests {
    use teloxide::types::Message;
    use crate::telegram::reply::resolve_session_model;
    use crate::telegram::reply::parse_model_directive;
    use crate::session::manager::SessionManager;
    use crate::config::CliConfig;
    use crate::session::key::SessionKey;

    #[tokio::test]
    async fn test_resolve_session_model_formatting() {
        let temp = tempfile::NamedTempFile::new().unwrap();
        let mgr = SessionManager::new(temp.path().to_path_buf(), 30, 4, false, "UTC".to_string(), None);
        
        let mut cfg = CliConfig::default();
        cfg.provider = "antigravity".to_string();
        cfg.model = Some("gemini-3.6-flash".to_string());
        cfg.effort = Some("high".to_string());
        
        let msg = serde_json::from_str::<Message>(r#"{
            "message_id": 1,
            "date": 123456,
            "chat": {"id": 123, "type": "private"},
            "text": "test"
        }"#).unwrap();
        
        let key = SessionKey::telegram(123, None);
        
        // Scenario 1: Session effort is None, should fallback to config.effort (high)
        let (mut sess, _) = mgr.resolve_session(&key, "antigravity", "gemini-3.6-flash").await.unwrap();
        sess.effort = None;
        mgr.update_session(&sess, 0.0, 0).await.unwrap();
        
        let model_str = resolve_session_model(&msg, &cfg, &mgr).await;
        assert_eq!(model_str, "gemini-3.6-flash (effort: high)");
        
        // Scenario 2: Session effort is Some("medium"), should use session effort
        sess.effort = Some("medium".to_string());
        mgr.update_session(&sess, 0.0, 0).await.unwrap();
        
        let model_str = resolve_session_model(&msg, &cfg, &mgr).await;
        assert_eq!(model_str, "gemini-3.6-flash (effort: medium)");
        
        // Scenario 3: Config effort is None and session effort is None
        cfg.effort = None;
        sess.effort = None;
        mgr.update_session(&sess, 0.0, 0).await.unwrap();
        
        let model_str = resolve_session_model(&msg, &cfg, &mgr).await;
        assert_eq!(model_str, "gemini-3.6-flash");
    }

    #[test]
    fn test_parse_model_directive_scenarios() {
        // Scenario 1: Standard format with @model prefix and effort flag
        let (model, effort, prompt) = parse_model_directive("@model gemini-3.6-flash --effort high write code");
        assert_eq!(model, Some("gemini-3.6-flash".to_string()));
        assert_eq!(effort, Some("high".to_string()));
        assert_eq!(prompt, "write code");

        // Scenario 2: Standard format with @model prefix and positional effort
        let (model, effort, prompt) = parse_model_directive("@model gemini-3.6-flash medium refactor this");
        assert_eq!(model, Some("gemini-3.6-flash".to_string()));
        assert_eq!(effort, Some("medium".to_string()));
        assert_eq!(prompt, "refactor this");

        // Scenario 3: Legacy format with suffix
        let (model, effort, prompt) = parse_model_directive("@model gemini-3.6-flash-high write a test");
        assert_eq!(model, Some("gemini-3.6-flash".to_string()));
        assert_eq!(effort, Some("high".to_string()));
        assert_eq!(prompt, "write a test");

        // Scenario 4: No model directive
        let (model, effort, prompt) = parse_model_directive("just plain user text");
        assert_eq!(model, None);
        assert_eq!(effort, None);
        assert_eq!(prompt, "just plain user text");
    }

    #[test]
    fn test_reply_to_message_with_expandable_blockquote() {
        use crate::telegram::reply::build_reply_prompt;

        let msg_json = r#"{
            "message_id": 2,
            "date": 123457,
            "chat": {"id": -1003735536827, "type": "supergroup"},
            "text": "my follow up question",
            "reply_to_message": {
                "message_id": 1,
                "date": 123456,
                "chat": {"id": -1003735536827, "type": "supergroup"},
                "text": "here is some collapsed code",
                "entities": [
                    {
                        "type": "expandable_blockquote",
                        "offset": 0,
                        "length": 27
                    }
                ]
            }
        }"#;
        let msg: Message = serde_json::from_str(msg_json).expect("should deserialize message with expandable_blockquote");
        let prompt = build_reply_prompt(&msg, "my follow up question");
        assert!(prompt.contains("> here is some collapsed code"));
        assert!(prompt.contains("my follow up question"));
    }

    #[test]
    fn test_reply_to_message_with_future_unknown_entity() {
        use crate::telegram::reply::build_reply_prompt;

        let msg_json = r#"{
            "message_id": 3,
            "date": 123458,
            "chat": {"id": -1003735536827, "type": "supergroup"},
            "text": "replying to future entity",
            "reply_to_message": {
                "message_id": 1,
                "date": 123456,
                "chat": {"id": -1003735536827, "type": "supergroup"},
                "text": "some text with new entity",
                "entities": [
                    {
                        "type": "super_future_unsupported_telegram_entity",
                        "offset": 0,
                        "length": 10
                    }
                ]
            }
        }"#;
        let msg: Message = serde_json::from_str(msg_json).expect("should deserialize message with unknown entity without crashing");
        let prompt = build_reply_prompt(&msg, "replying to future entity");
        assert!(prompt.contains("> some text with new entity"));
    }
}
