//! # Session Manager Tests
//!
//! This module contains integration and unit tests validating SessionManager persistence,
//! migration of legacy keys, and daily timezone-based expiration logic.

use super::manager::SessionManager;
use super::data::SessionData;
use super::key::SessionKey;
use tempfile::NamedTempFile;
use chrono::{Utc, Duration};

fn create_temp_manager(
    temp_file: &NamedTempFile,
    idle: i64,
    reset_hour: u32,
    reset_enabled: bool,
    tz: &str,
) -> SessionManager {
    SessionManager::new(
        temp_file.path().to_path_buf(),
        idle,
        reset_hour,
        reset_enabled,
        tz.to_string(),
        None,
    )
}

#[tokio::test]
async fn test_resolve_creates_new_session() {
    let temp = NamedTempFile::new().unwrap();
    let mgr = create_temp_manager(&temp, 30, 4, false, "UTC");
    let key = SessionKey::telegram(1, None);

    let (s, is_new) = mgr.resolve_session(&key, "claude", "opus").await.unwrap();
    assert!(is_new);
    assert_eq!(s.chat_id, 1);
    assert_eq!(s.provider, "claude");
    assert_eq!(s.model, "opus");
}

#[tokio::test]
async fn test_resolve_reuses_fresh_session() {
    let temp = NamedTempFile::new().unwrap();
    let mgr = create_temp_manager(&temp, 30, 4, false, "UTC");
    let key = SessionKey::telegram(1, None);

    let (s1, is_new1) = mgr.resolve_session(&key, "claude", "opus").await.unwrap();
    assert!(is_new1);
    
    // Simulate updating session with CLI response session ID
    let mut updated = s1.clone();
    updated.set_session_id("claude", "sess-123");
    mgr.update_session(&updated, 0.0, 0).await.unwrap();

    let (s2, is_new2) = mgr.resolve_session(&key, "claude", "opus").await.unwrap();
    assert!(!is_new2);
    assert_eq!(s2.get_session_id("claude"), "sess-123");
}

#[tokio::test]
async fn test_session_expires_after_idle_timeout() {
    let temp = NamedTempFile::new().unwrap();
    let mgr = create_temp_manager(&temp, 30, 4, false, "UTC");
    let key = SessionKey::telegram(1, None);

    let (s1, _) = mgr.resolve_session(&key, "claude", "opus").await.unwrap();
    
    // Check fresh
    assert!(mgr.is_fresh(&s1));

    // Make it expired
    let mut stale = s1.clone();
    let past = Utc::now() - Duration::minutes(31);
    stale.last_active = past.to_rfc3339();
    
    assert!(!mgr.is_fresh(&stale));
}

#[tokio::test]
async fn test_session_expires_at_daily_reset() {
    let temp = NamedTempFile::new().unwrap();
    // Daily reset enabled at 04:00 AM UTC
    let mgr = create_temp_manager(&temp, 0, 4, true, "UTC");

    let mut session = SessionData::new(1, "tg".to_string(), None, "claude".to_string(), "opus".to_string());
    
    // Set last active to 5 minutes before today's 04:00 AM UTC reset
    let now = Utc::now();
    let today_reset = now.date_naive().and_hms_opt(4, 0, 0).unwrap().and_local_timezone(Utc).unwrap();

    let last_active_time = if now >= today_reset {
        today_reset - Duration::minutes(5)
    } else {
        today_reset - Duration::days(1) - Duration::minutes(5)
    };

    session.last_active = last_active_time.to_rfc3339();

    // Since today's reset point has passed compared to last_active_time, the session should not be fresh
    assert!(!mgr.is_fresh(&session));
}

#[tokio::test]
async fn test_persistence_across_instances() {
    let temp = NamedTempFile::new().unwrap();
    let mgr1 = create_temp_manager(&temp, 30, 4, false, "UTC");
    let key = SessionKey::telegram(1, None);

    let (s1, _) = mgr1.resolve_session(&key, "claude", "opus").await.unwrap();
    let mut updated = s1.clone();
    updated.set_session_id("claude", "persisted-123");
    mgr1.update_session(&updated, 0.05, 100).await.unwrap();

    let mgr2 = create_temp_manager(&temp, 30, 4, false, "UTC");
    let (s2, is_new) = mgr2.resolve_session(&key, "claude", "opus").await.unwrap();
    assert!(!is_new);
    assert_eq!(s2.get_session_id("claude"), "persisted-123");
    
    let ps = s2.provider_sessions.get("claude").unwrap();
    assert_eq!(ps.message_count, 1);
    assert_eq!(ps.total_cost_usd, 0.05);
    assert_eq!(ps.total_tokens, 100);
}

#[tokio::test]
async fn test_legacy_key_migrations() {
    let temp = NamedTempFile::new().unwrap();
    let path = temp.path().to_path_buf();
    
    // Write legacy data manually
    let legacy_json = r#"{
        "6087616160": {
            "chat_id": 6087616160,
            "provider": "claude",
            "model": "opus",
            "session_id": "legacy-sess",
            "message_count": 5,
            "total_cost_usd": 0.12,
            "total_tokens": 1200
        },
        "123:45": {
            "chat_id": 123,
            "topic_id": 45,
            "provider": "claude",
            "model": "opus"
        }
    }"#;
    std::fs::write(&path, legacy_json).unwrap();

    let mgr = SessionManager::new(path.clone(), 30, 4, false, "UTC".to_string(), None);
    let sessions = mgr.load().unwrap();

    // Check migrated keys
    assert!(sessions.contains_key("tg:6087616160"));
    assert!(sessions.contains_key("tg:123:45"));

    // Check legacy metrics migration
    let s1 = sessions.get("tg:6087616160").unwrap();
    assert_eq!(s1.transport, "tg");
    assert_eq!(s1.chat_id, 6087616160);
    assert_eq!(s1.provider, "claude");
    assert_eq!(s1.model, "opus");
    
    let ps = s1.provider_sessions.get("claude").unwrap();
    assert_eq!(ps.session_id, "legacy-sess");
    assert_eq!(ps.message_count, 5);
    assert_eq!(ps.total_cost_usd, 0.12);
    assert_eq!(ps.total_tokens, 1200);

    let s2 = sessions.get("tg:123:45").unwrap();
    assert_eq!(s2.chat_id, 123);
    assert_eq!(s2.topic_id, Some(45));
    assert_eq!(s2.transport, "tg");
}

#[tokio::test]
async fn test_reset_provider_session() {
    let temp = NamedTempFile::new().unwrap();
    let mgr = create_temp_manager(&temp, 30, 4, false, "UTC");
    let key = SessionKey::telegram(1, None);

    let (s1, _) = mgr.resolve_session(&key, "antigravity", "opus").await.unwrap();
    let mut updated = s1.clone();
    updated.set_session_id("antigravity", "active-sess-id");
    mgr.update_session(&updated, 0.05, 100).await.unwrap();

    // Verify it exists first
    let s_before = mgr.get_active(&key).await.unwrap().unwrap();
    assert_eq!(s_before.get_session_id("antigravity"), "active-sess-id");

    // Call reset
    let reset_sess = mgr.reset_provider_session(&key, "antigravity", "gemini").await.unwrap();
    assert_eq!(reset_sess.get_session_id("antigravity"), "");
    assert_eq!(reset_sess.provider, "antigravity");
    assert_eq!(reset_sess.model, "gemini");

    // Verify persisted
    let s_after = mgr.get_active(&key).await.unwrap().unwrap();
    assert_eq!(s_after.get_session_id("antigravity"), "");
}

