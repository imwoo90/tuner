//! # Extra Telegram Reply Tests
//!
//! Supplemental tests for resolve_session_model and parse_model_directive.
#![allow(clippy::field_reassign_with_default)]

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
        mgr.update_session(&sess).await.unwrap();
        
        let model_str = resolve_session_model(&msg, &cfg, &mgr).await;
        assert_eq!(model_str, "gemini-4-hydrogen (effort: high)");
        
        // Scenario 2: Session effort is Some("medium"), should use session effort
        sess.effort = Some("medium".to_string());
        mgr.update_session(&sess).await.unwrap();
        
        let model_str = resolve_session_model(&msg, &cfg, &mgr).await;
        assert_eq!(model_str, "gemini-4-hydrogen (effort: medium)");
        
        // Scenario 3: Config effort is None and session effort is None
        cfg.effort = None;
        sess.effort = None;
        mgr.update_session(&sess).await.unwrap();
        
        let model_str = resolve_session_model(&msg, &cfg, &mgr).await;
        assert_eq!(model_str, "gemini-4-hydrogen");

        // Scenario 4: Effortless orchestrator ignores global config.effort ("high")
        cfg.effort = Some("high".to_string());
        sess.model = "gemini-pro-orchestrator".to_string();
        sess.effort = None;
        mgr.update_session(&sess).await.unwrap();

        let orch_str = resolve_session_model(&msg, &cfg, &mgr).await;
        assert_eq!(orch_str, "gemini-pro-orchestrator");
    }

    #[test]
    fn test_parse_model_directive_scenarios() {
        // Scenario 1: Standard format with @model prefix and effort flag (normalizes legacy 3.6 -> 4-hydrogen)
        let (model, effort, prompt) = parse_model_directive("@model gemini-3.6-flash --effort high write code");
        assert_eq!(model, Some("gemini-4-hydrogen".to_string()));
        assert_eq!(effort, Some("high".to_string()));
        assert_eq!(prompt, "write code");

        // Scenario 2: Standard format with @model prefix and positional effort
        let (model, effort, prompt) = parse_model_directive("@model gemini-3.6-flash medium refactor this");
        assert_eq!(model, Some("gemini-4-hydrogen".to_string()));
        assert_eq!(effort, Some("medium".to_string()));
        assert_eq!(prompt, "refactor this");

        // Scenario 3: Legacy format with suffix
        let (model, effort, prompt) = parse_model_directive("@model gemini-3.6-flash-high write a test");
        assert_eq!(model, Some("gemini-4-hydrogen".to_string()));
        assert_eq!(effort, Some("high".to_string()));
        assert_eq!(prompt, "write a test");

        // Scenario 4: No model directive
        let (model, effort, prompt) = parse_model_directive("just plain user text");
        assert_eq!(model, None);
        assert_eq!(effort, None);
        assert_eq!(prompt, "just plain user text");

        // Scenario 5: Orchestrator directive strips unsupported effort flags/suffixes
        let (om, oe, op) = parse_model_directive("@model gemini-pro-orchestrator-high coordinate tasks");
        assert_eq!(om.as_deref(), Some("gemini-pro-orchestrator"));
        assert_eq!(oe, None);
        assert_eq!(op, "coordinate tasks");

        let (om2, oe2, op2) = parse_model_directive("@orchestrator coordinate swarm");
        assert_eq!(om2.as_deref(), Some("gemini-pro-orchestrator"));
        assert_eq!(oe2, None);
        assert_eq!(op2, "coordinate swarm");
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
        // In teloxide 0.17, messages with unknown entity types gracefully fallback to MessageKind::Empty,
        // ensuring the daemon never crashes during deserialization.
        assert_eq!(prompt, "replying to future entity");
    }

    #[test]
    fn test_cli_config_session_expiration_defaults_and_deserialization() {
        let json = r#"{
            "idle_timeout_minutes": 120,
            "daily_reset_hour": 5,
            "daily_reset_enabled": true,
            "max_session_messages": 100,
            "user_timezone": "Asia/Seoul"
        }"#;
        let cfg: CliConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.idle_timeout_minutes, 120);
        assert_eq!(cfg.daily_reset_hour, 5);
        assert!(cfg.daily_reset_enabled);
        assert_eq!(cfg.max_session_messages, Some(100));
        assert_eq!(cfg.user_timezone, Some("Asia/Seoul".to_string()));

        let default_cfg = CliConfig::default();
        assert_eq!(default_cfg.idle_timeout_minutes, 0);
        assert_eq!(default_cfg.daily_reset_hour, 4);
        assert!(!default_cfg.daily_reset_enabled);
        assert_eq!(default_cfg.max_session_messages, None);
    }

    #[test]
    fn test_is_message_not_modified() {
        use teloxide::RequestError;
        use teloxide::ApiError;
        let err_api = RequestError::Api(ApiError::MessageNotModified);
        assert!(crate::messenger::telegram::stream::is_message_not_modified(&err_api));

        let err_str = RequestError::Io(std::io::Error::other("Bad Request: message is not modified").into());
        assert!(crate::messenger::telegram::stream::is_message_not_modified(&err_str));

        let err_other = RequestError::Api(ApiError::ChatNotFound);
        assert!(!crate::messenger::telegram::stream::is_message_not_modified(&err_other));
    }

    #[tokio::test]
    async fn test_build_sessions_binds_config_and_enforces_freshness() {
        let temp = tempfile::NamedTempFile::new().unwrap();
        let cache = std::sync::Arc::new(crate::telegram::TopicNameCache::new());

        let mut cfg = CliConfig::default();
        cfg.idle_timeout_minutes = 60;
        cfg.daily_reset_hour = 4;
        cfg.daily_reset_enabled = false;
        cfg.max_session_messages = Some(5);
        cfg.user_timezone = Some("UTC".to_string());

        let mgr = crate::messenger::telegram::runner::build_sessions(temp.path().to_path_buf(), cache, &cfg);

        let now = chrono::Utc::now();
        let mut sess = crate::session::data::SessionData::default();
        sess.last_active = (now - chrono::Duration::minutes(30)).to_rfc3339();

        let mut ps = crate::session::data::ProviderSessionData::default();
        ps.message_count = 3;
        sess.provider_sessions.insert(sess.provider.clone(), ps);

        // Fresh session: updated 30 min ago with 3 messages
        assert!(mgr.is_fresh(&sess), "Session under idle and message bounds should be fresh");

        // Expired session by idle timeout: updated 90 min ago (limit 60)
        sess.last_active = (now - chrono::Duration::minutes(90)).to_rfc3339();
        assert!(!mgr.is_fresh(&sess), "Session exceeding idle_timeout_minutes must expire");

        // Expired session by message count: updated 10 min ago but 6 messages (limit 5)
        sess.last_active = (now - chrono::Duration::minutes(10)).to_rfc3339();
        sess.provider_sessions.get_mut(&sess.provider).unwrap().message_count = 6;
        assert!(!mgr.is_fresh(&sess), "Session exceeding max_session_messages must expire");
    }

    #[test]
    fn test_provider_aware_directives_and_provider_switch_clamping() {
        use crate::messenger::telegram::reply_directives::{
            parse_model_directive_for_provider, parse_model_effort_args_for_provider,
        };
        use crate::session::data::SessionData;

        let (cm, ce, cp) = parse_model_directive_for_provider("claude", "@opus write tests");
        assert_eq!(cm.as_deref(), Some("claude-opus-4-8"));
        assert_eq!(ce, None);
        assert_eq!(cp, "write tests");

        let (hm, he, hp) = parse_model_directive_for_provider("claude", "@haiku quick");
        assert_eq!(hm.as_deref(), Some("claude-haiku-4-5"));
        assert_eq!(he, None);
        assert_eq!(hp, "quick");

        let (xm, xe, xp) = parse_model_directive_for_provider("codex", "@pro do work");
        assert_eq!(xm, None);
        assert_eq!(xe, None);
        assert_eq!(xp, "@pro do work");

        let (am, ae, ap) =
            parse_model_directive_for_provider("antigravity", "@model gemini-4-argon --effort turbo run");
        assert_eq!(am, None);
        assert_eq!(ae, None);
        assert_eq!(ap, "@model gemini-4-argon --effort turbo run");

        let (am2, ae2, ap2) =
            parse_model_directive_for_provider("antigravity", "@model gemini-4-argon --effort max run");
        assert_eq!(am2.as_deref(), Some("gemini-4-argon"));
        assert_eq!(ae2.as_deref(), Some("high"));
        assert_eq!(ap2, "run");

        assert_eq!(
            parse_model_effort_args_for_provider("claude", "haiku", Some("high")),
            ("claude-haiku-4-5".to_string(), None)
        );
        assert_eq!(
            parse_model_effort_args_for_provider("claude", "sonnet --effort minimal", None),
            ("claude-sonnet-5".to_string(), Some("low".to_string()))
        );
        assert_eq!(
            parse_model_effort_args_for_provider("claude", "--effort=high sonnet", None),
            ("claude-sonnet-5".to_string(), Some("high".to_string()))
        );
        assert_eq!(
            parse_model_effort_args_for_provider("codex", "luna --effort max", None),
            ("gpt-6-luna".to_string(), Some("xhigh".to_string()))
        );
        assert_eq!(
            parse_model_effort_args_for_provider("antigravity", "pro", Some("low")),
            ("gemini-4-argon".to_string(), Some("low".to_string()))
        );
        assert_eq!(
            parse_model_effort_args_for_provider("antigravity", "gpt-oss-120b-medium", Some("high")),
            ("gpt-oss-120b-medium".to_string(), None)
        );

        let mut s = SessionData::new(1, "tg".into(), None, "claude".into(), "claude-fable-5".into());
        s.effort = Some("max".into());
        s.apply_provider_switch("antigravity");
        assert_eq!(s.model, "gemini-4-hydrogen");
        assert_eq!(s.effort.as_deref(), Some("xhigh"));

        s.model = "o3".into();
        s.effort = Some("minimal".into());
        s.apply_provider_switch("claude");
        assert_eq!(s.model, "claude-fable-5");
        assert_eq!(s.effort.as_deref(), Some("low"));

        s.effort = Some("max".into());
        s.apply_provider_switch("codex");
        assert_eq!(s.model, "gpt-6-astra");
        assert_eq!(s.effort.as_deref(), Some("xhigh"));
    }

    #[test]
    fn test_round3_shortcut_isolation_effort_inheritance_and_suffix_switch() {
        use crate::messenger::telegram::commands_model::group_discovered_models;
        use crate::messenger::telegram::reply_directives::{
            DirectiveOutcome, parse_model_directive_for_provider, resolve_directive_for_provider,
            tokenize_model_command_args,
        };

        // Claude/Codex-only aliases must not hijack `@fable` / `@astra` on Antigravity
        assert_eq!(parse_model_directive_for_provider("antigravity", "@fable hello").0, None);
        assert_eq!(parse_model_directive_for_provider("antigravity", "@astra hello").0, None);
        assert_eq!(
            resolve_directive_for_provider("antigravity", None, "@model gemini-4-argon --effort turbo run"),
            DirectiveOutcome::InvalidEffort
        );
        assert!(tokenize_model_command_args("sonnet --effort=turbo").is_err());
        assert_eq!(
            tokenize_model_command_args("--effort high sonnet").unwrap(),
            ("sonnet".to_string(), Some("high".to_string()))
        );

        // Single-variant `gpt-oss-120b-medium` stays intact in `/model` grouping without 1-button effort split
        let grouped = group_discovered_models(&[
            "gemini-4-carbon-high".to_string(),
            "gemini-4-carbon-low".to_string(),
            "gpt-oss-120b-medium".to_string(),
        ]);
        assert_eq!(grouped[0].0, "gemini-4-carbon");
        assert_eq!(grouped[1], ("gpt-oss-120b-medium".to_string(), Vec::new()));
    }

    #[test]
    fn test_round3_orchestrator_aliases_and_suffix_provider_switches() {
        use crate::messenger::telegram::reply_directives::{
            DirectiveOutcome, parse_model_directive_for_provider, resolve_directive_for_provider,
        };
        use crate::session::data::SessionData;

        // Orchestrator variant aliases resolve cleanly with `None` effort
        let (oa, oe, op) = parse_model_directive_for_provider("antigravity", "@orchestrator-argon coordinate");
        assert_eq!(oa.as_deref(), Some("gemini-pro-orchestrator-argon"));
        assert_eq!(oe, None);
        assert_eq!(op, "coordinate");

        // Inline `@argon` preserves `prev_effort` (`None` or `Some("low")`) without forcing `"high"`
        assert_eq!(
            resolve_directive_for_provider("antigravity", None, "@argon plan"),
            DirectiveOutcome::Resolved("gemini-4-argon".to_string(), None, "plan")
        );
        assert_eq!(
            resolve_directive_for_provider("antigravity", Some("low"), "@argon plan"),
            DirectiveOutcome::Resolved("gemini-4-argon".to_string(), Some("low".to_string()), "plan")
        );

        // Suffixed `claude-opus-5-5-high`: incompatible on `claude`, normalized on `antigravity`
        let mut s1 = SessionData::new(1, "tg".into(), None, "antigravity".into(), "claude-opus-5-5-high".into());
        s1.apply_provider_switch("claude");
        assert_eq!(s1.model, "claude-fable-5");

        let mut s2 = SessionData::new(1, "tg".into(), None, "claude".into(), "claude-opus-5-5-high".into());
        s2.apply_provider_switch("antigravity");
        assert_eq!(s2.model, "claude-opus-5-5");
        assert_eq!(s2.effort.as_deref(), Some("high"));

        let mut s3 = SessionData::new(1, "tg".into(), None, "claude".into(), "codex-mini".into());
        s3.apply_provider_switch("codex");
        assert_eq!(s3.model, "codex-mini");
    }
}
