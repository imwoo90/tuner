//! # Session Manager Tests
//!
//! This module contains integration and unit tests validating SessionManager persistence,
//! migration of legacy keys, and daily timezone-based expiration logic.

use super::manager::SessionManager;
use super::data::SessionData;
use super::key::SessionKey;
use tempfile::NamedTempFile;
use chrono::{Utc, Duration};

fn create_temp_manager(t: &NamedTempFile, idle: i64, hr: u32, en: bool, tz: &str) -> SessionManager {
    SessionManager::new(t.path().to_path_buf(), idle, hr, en, tz.to_string(), None)
}

#[tokio::test]
async fn test_resolve_creates_new_session() {
    let t = NamedTempFile::new().unwrap();
    let m = create_temp_manager(&t, 30, 4, false, "UTC");
    let (s, new) = m.resolve_session(&SessionKey::telegram(1, None), "claude", "opus").await.unwrap();
    assert!(new);
    assert_eq!(s.chat_id, 1);
    assert_eq!(s.provider, "claude");
    assert_eq!(s.model, "opus");
}

#[tokio::test]
async fn test_resolve_reuses_fresh_session() {
    let t = NamedTempFile::new().unwrap();
    let m = create_temp_manager(&t, 30, 4, false, "UTC");
    let k = SessionKey::telegram(1, None);
    let (s1, new1) = m.resolve_session(&k, "claude", "opus").await.unwrap();
    assert!(new1);
    let mut u = s1.clone();
    u.set_session_id("claude", "sess-123");
    m.update_session(&u, 0.0, 0).await.unwrap();
    let (s2, new2) = m.resolve_session(&k, "claude", "opus").await.unwrap();
    assert!(!new2);
    assert_eq!(s2.get_session_id("claude"), "sess-123");
}

#[tokio::test]
async fn test_session_expires_after_idle_timeout() {
    let t = NamedTempFile::new().unwrap();
    let m = create_temp_manager(&t, 30, 4, false, "UTC");
    let (s1, _) = m.resolve_session(&SessionKey::telegram(1, None), "claude", "opus").await.unwrap();
    assert!(m.is_fresh(&s1));
    let mut stale = s1.clone();
    stale.last_active = (Utc::now() - Duration::minutes(31)).to_rfc3339();
    assert!(!m.is_fresh(&stale));
}

#[tokio::test]
async fn test_session_expires_at_daily_reset() {
    let t = NamedTempFile::new().unwrap();
    let m = create_temp_manager(&t, 0, 4, true, "UTC");
    let mut s = SessionData::new(1, "tg".to_string(), None, "claude".to_string(), "opus".to_string());
    let now = Utc::now();
    let today = now.date_naive().and_hms_opt(4, 0, 0).unwrap().and_local_timezone(Utc).unwrap();
    let last = if now >= today { today - Duration::minutes(5) } else { today - Duration::days(1) - Duration::minutes(5) };
    s.last_active = last.to_rfc3339();
    assert!(!m.is_fresh(&s));
}

#[tokio::test]
async fn test_persistence_across_instances() {
    let t = NamedTempFile::new().unwrap();
    let m1 = create_temp_manager(&t, 30, 4, false, "UTC");
    let k = SessionKey::telegram(1, None);
    let (s1, _) = m1.resolve_session(&k, "claude", "opus").await.unwrap();
    let mut u = s1.clone();
    u.set_session_id("claude", "persisted-123");
    m1.update_session(&u, 0.05, 100).await.unwrap();
    let m2 = create_temp_manager(&t, 30, 4, false, "UTC");
    let (s2, new) = m2.resolve_session(&k, "claude", "opus").await.unwrap();
    assert!(!new);
    assert_eq!(s2.get_session_id("claude"), "persisted-123");
    let ps = s2.provider_sessions.get("claude").unwrap();
    assert_eq!(ps.message_count, 1);
    assert_eq!(ps.total_cost_usd, 0.05);
    assert_eq!(ps.total_tokens, 100);
}

#[tokio::test]
async fn test_legacy_key_migrations() {
    let t = NamedTempFile::new().unwrap();
    let legacy = r#"{"6087616160":{"chat_id":6087616160,"provider":"claude","model":"opus","session_id":"legacy-sess","message_count":5,"total_cost_usd":0.12,"total_tokens":1200},"123:45":{"chat_id":123,"topic_id":45,"provider":"claude","model":"opus"}}"#;
    std::fs::write(t.path(), legacy).unwrap();
    let m = SessionManager::new(t.path().to_path_buf(), 30, 4, false, "UTC".to_string(), None);
    let ss = m.load().unwrap();
    assert!(ss.contains_key("tg:6087616160"));
    assert!(ss.contains_key("tg:123:45"));
    let s1 = ss.get("tg:6087616160").unwrap();
    assert_eq!(s1.transport, "tg");
    assert_eq!(s1.chat_id, 6087616160);
    assert_eq!(s1.provider, "claude");
    assert_eq!(s1.model, "opus");
    let ps = s1.provider_sessions.get("claude").unwrap();
    assert_eq!(ps.session_id, "legacy-sess");
    assert_eq!(ps.message_count, 5);
    assert_eq!(ps.total_cost_usd, 0.12);
    assert_eq!(ps.total_tokens, 1200);
    let s2 = ss.get("tg:123:45").unwrap();
    assert_eq!(s2.chat_id, 123);
    assert_eq!(s2.topic_id, Some(45));
    assert_eq!(s2.transport, "tg");
}

#[tokio::test]
async fn test_reset_provider_session() {
    let t = NamedTempFile::new().unwrap();
    let m = create_temp_manager(&t, 30, 4, false, "UTC");
    let k = SessionKey::telegram(1, None);
    let (s1, _) = m.resolve_session(&k, "antigravity", "opus").await.unwrap();
    let mut u = s1.clone();
    u.set_session_id("antigravity", "active-sess-id");
    m.update_session(&u, 0.05, 100).await.unwrap();
    let s_before = m.get_active(&k).await.unwrap().unwrap();
    assert_eq!(s_before.get_session_id("antigravity"), "active-sess-id");
    let rs = m.reset_provider_session(&k, "antigravity", "gemini").await.unwrap();
    assert_eq!(rs.get_session_id("antigravity"), "");
    assert_eq!(rs.provider, "antigravity");
    assert_eq!(rs.model, "gemini");
    let s_after = m.get_active(&k).await.unwrap().unwrap();
    assert_eq!(s_after.get_session_id("antigravity"), "");
}

#[test]
fn test_serialization_ignores_legacy_fields() {
    let mut d = SessionData::new(123, "tg".to_string(), None, "claude".to_string(), "opus".to_string());
    d.session_id = Some("legacy-sess".to_string());
    d.message_count = Some(10);
    d.total_cost_usd = Some(1.23);
    d.total_tokens = Some(456);
    let ser = serde_json::to_string(&d).unwrap();
    let val: serde_json::Value = serde_json::from_str(&ser).unwrap();
    assert!(val.get("session_id").is_none());
    assert!(val.get("message_count").is_none());
    assert!(val.get("total_cost_usd").is_none());
    assert!(val.get("total_tokens").is_none());
}

#[tokio::test]
async fn test_corrupt_session_file_recovers() {
    let t = NamedTempFile::new().unwrap();
    std::fs::write(t.path(), "not a valid json {{{{ }").unwrap();
    let m = create_temp_manager(&t, 30, 4, false, "UTC");
    let ss = m.load();
    assert!(ss.is_ok());
    assert!(ss.unwrap().is_empty());
}

#[tokio::test]
async fn test_message_limit_expiration() {
    let t = NamedTempFile::new().unwrap();
    let m = SessionManager::new(t.path().to_path_buf(), 30, 4, false, "UTC".to_string(), Some(2));
    let k = SessionKey::telegram(1, None);
    let (s1, _) = m.resolve_session(&k, "claude", "opus").await.unwrap();
    assert!(m.is_fresh(&s1));
    let s2 = m.update_session(&s1, 0.0, 0).await.unwrap();
    assert!(m.is_fresh(&s2));
    let s3 = m.update_session(&s2, 0.0, 0).await.unwrap();
    assert!(!m.is_fresh(&s3));
}

#[tokio::test]
async fn test_timezone_aware_daily_resets() {
    let la = chrono::DateTime::parse_from_rfc3339("2026-07-10T20:55:00Z").unwrap().with_timezone(&Utc);
    let now = chrono::DateTime::parse_from_rfc3339("2026-07-10T21:05:00Z").unwrap().with_timezone(&Utc);
    assert!(super::freshness::has_crossed_daily_reset(&la, &now, "Asia/Seoul", 6));
    let la_fresh = chrono::DateTime::parse_from_rfc3339("2026-07-10T21:01:00Z").unwrap().with_timezone(&Utc);
    assert!(!super::freshness::has_crossed_daily_reset(&la_fresh, &now, "Asia/Seoul", 6));
}

#[tokio::test]
async fn test_topic_name_resolver() {
    let t = NamedTempFile::new().unwrap();
    let m = create_temp_manager(&t, 30, 4, false, "UTC")
        .with_topic_resolver(std::sync::Arc::new(|cid, tid| Some(format!("Resolved-{cid}-{tid}"))));
    let (s, _) = m.resolve_session(&SessionKey::telegram(42, Some(99)), "claude", "opus").await.unwrap();
    assert_eq!(s.topic_name, Some("Resolved-42-99".to_string()));
}

#[tokio::test]
async fn test_query_methods() {
    let t = NamedTempFile::new().unwrap();
    let m = create_temp_manager(&t, 30, 4, false, "UTC");
    let k1 = SessionKey::telegram(42, None);
    let k2 = SessionKey::telegram(42, Some(1));
    let k3 = SessionKey::telegram(43, None);
    m.resolve_session(&k1, "claude", "opus").await.unwrap();
    m.resolve_session(&k2, "claude", "opus").await.unwrap();
    m.resolve_session(&k3, "claude", "opus").await.unwrap();
    assert_eq!(m.list_all().await.unwrap().len(), 3);
    assert_eq!(m.list_active_for_chat(42).await.unwrap().len(), 2);
}

#[tokio::test]
async fn test_model_switch_override_preservation() {
    let t = NamedTempFile::new().unwrap();
    let m = create_temp_manager(&t, 30, 4, false, "UTC");
    let k = SessionKey::telegram(1, None);
    let (s1, _) = m.resolve_session(&k, "claude", "opus").await.unwrap();
    let mut ov = s1.clone();
    ov.provider = "openai".to_string();
    ov.model = "gpt-4".to_string();
    ov.set_session_id("openai", "sess-openai-123");
    m.update_session(&ov, 0.0, 0).await.unwrap();
    let (s2, new) = m.resolve_session(&k, "claude", "opus").await.unwrap();
    assert!(!new);
    assert_eq!(s2.provider, "openai");
    assert_eq!(s2.model, "gpt-4");
}

#[tokio::test]
async fn test_preserve_session_identity() {
    let t = NamedTempFile::new().unwrap();
    let m = create_temp_manager(&t, 30, 4, false, "UTC");
    let (s1, _) = m.resolve_session(&SessionKey::telegram(1, None), "claude", "opus").await.unwrap();
    let mut u = s1.clone();
    u.set_session_id("claude", "sess-xyz");
    u.language = Some("ko".to_string());
    let res = m.preserve_session_identity(&u).await.unwrap();
    assert_eq!(res.get_session_id("claude"), "sess-xyz");
    assert_eq!(res.language, Some("ko".to_string()));
    assert_eq!(res.provider_sessions.get("claude").unwrap().message_count, 0);
}

