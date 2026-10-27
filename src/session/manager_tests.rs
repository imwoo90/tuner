//! # Session Manager Tests
//!
//! This module contains integration and unit tests validating SessionManager persistence,
//! session rotation, and daily timezone-based expiration logic.

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
    let (s, new) = m.resolve_session(&SessionKey::telegram(1, None), "antigravity", "gemini-3.8-flash").await.unwrap();
    assert!(new);
    assert_eq!(s.chat_id, 1);
    assert_eq!(s.provider, "antigravity");
    assert_eq!(s.model, "gemini-3.8-flash");
}

#[tokio::test]
async fn test_resolve_reuses_fresh_session() {
    let t = NamedTempFile::new().unwrap();
    let m = create_temp_manager(&t, 30, 4, false, "UTC");
    let k = SessionKey::telegram(1, None);
    let (s1, new1) = m.resolve_session(&k, "antigravity", "gemini-3.8-flash").await.unwrap();
    assert!(new1);
    let mut u = s1.clone();
    u.set_session_id("antigravity", "sess-123");
    m.update_session(&u).await.unwrap();
    let (s2, new2) = m.resolve_session(&k, "antigravity", "gemini-3.8-flash").await.unwrap();
    assert!(!new2);
    assert_eq!(s2.get_session_id("antigravity"), "sess-123");
}

#[tokio::test]
async fn test_session_expires_after_idle_timeout() {
    let t = NamedTempFile::new().unwrap();
    let m = create_temp_manager(&t, 30, 4, false, "UTC");
    let (s1, _) = m.resolve_session(&SessionKey::telegram(1, None), "antigravity", "gemini-3.8-flash").await.unwrap();
    assert!(m.is_fresh(&s1));
    let mut stale = s1.clone();
    stale.last_active = (Utc::now() - Duration::minutes(31)).to_rfc3339();
    assert!(!m.is_fresh(&stale));
}

#[tokio::test]
async fn test_session_expires_at_daily_reset() {
    let t = NamedTempFile::new().unwrap();
    let m = create_temp_manager(&t, 0, 4, true, "UTC");
    let mut s = SessionData::new(1, "tg".to_string(), None, "antigravity".to_string(), "gemini-3.8-flash".to_string());
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
    let (s1, _) = m1.resolve_session(&k, "antigravity", "gemini-3.8-flash").await.unwrap();
    let mut u = s1.clone();
    u.set_session_id("antigravity", "persisted-123");
    m1.update_session(&u).await.unwrap();
    let m2 = create_temp_manager(&t, 30, 4, false, "UTC");
    let (s2, new) = m2.resolve_session(&k, "antigravity", "gemini-3.8-flash").await.unwrap();
    assert!(!new);
    assert_eq!(s2.get_session_id("antigravity"), "persisted-123");
    let ps = s2.provider_sessions.get("antigravity").unwrap();
    assert_eq!(ps.message_count, 1);
}

#[tokio::test]
async fn test_reset_provider_session() {
    let t = NamedTempFile::new().unwrap();
    let m = create_temp_manager(&t, 30, 4, false, "UTC");
    let k = SessionKey::telegram(1, None);
    let (s1, _) = m.resolve_session(&k, "antigravity", "gemini-4-argon").await.unwrap();
    let mut u = s1.clone();
    u.set_session_id("antigravity", "active-sess-id");
    m.update_session(&u).await.unwrap();
    let s_before = m.get_active(&k).await.unwrap().unwrap();
    assert_eq!(s_before.get_session_id("antigravity"), "active-sess-id");
    let rs = m.reset_provider_session(&k, "antigravity", "gemini-3.8-flash").await.unwrap();
    assert_eq!(rs.get_session_id("antigravity"), "");
    assert_eq!(rs.provider, "antigravity");
    assert_eq!(rs.model, "gemini-3.8-flash");
    let s_after = m.get_active(&k).await.unwrap().unwrap();
    assert_eq!(s_after.get_session_id("antigravity"), "");
    assert_eq!(s_after.get_past_session_ids("antigravity"), vec!["active-sess-id".to_string()]);
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
    let (s1, _) = m.resolve_session(&k, "antigravity", "gemini-3.8-flash").await.unwrap();
    assert!(m.is_fresh(&s1));
    let s2 = m.update_session(&s1).await.unwrap();
    assert!(m.is_fresh(&s2));
    let s3 = m.update_session(&s2).await.unwrap();
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
    let (s, _) = m.resolve_session(&SessionKey::telegram(42, Some(99)), "antigravity", "gemini-3.8-flash").await.unwrap();
    assert_eq!(s.topic_name, Some("Resolved-42-99".to_string()));
}

#[tokio::test]
async fn test_query_methods() {
    let t = NamedTempFile::new().unwrap();
    let m = create_temp_manager(&t, 30, 4, false, "UTC");
    let k1 = SessionKey::telegram(42, None);
    let k2 = SessionKey::telegram(42, Some(1));
    let k3 = SessionKey::telegram(43, None);
    m.resolve_session(&k1, "antigravity", "gemini-3.8-flash").await.unwrap();
    m.resolve_session(&k2, "antigravity", "gemini-3.8-flash").await.unwrap();
    m.resolve_session(&k3, "antigravity", "gemini-3.8-flash").await.unwrap();
    assert_eq!(m.list_all().await.unwrap().len(), 3);
}

#[tokio::test]
async fn test_model_switch_override_preservation() {
    let t = NamedTempFile::new().unwrap();
    let m = create_temp_manager(&t, 30, 4, false, "UTC");
    let k = SessionKey::telegram(1, None);
    let (s1, _) = m.resolve_session(&k, "antigravity", "gemini-3.8-flash").await.unwrap();
    let mut ov = s1.clone();
    ov.model = "gemini-4-argon".to_string();
    ov.set_session_id("antigravity", "sess-argon-123");
    m.preserve_session_identity(&ov).await.unwrap();
    let (s2, new) = m.resolve_session(&k, "antigravity", "gemini-3.8-flash").await.unwrap();
    assert!(!new);
    assert_eq!(s2.model, "gemini-4-argon");
}

#[tokio::test]
async fn test_preserve_session_identity_preserves_past_ids_progress_and_clears_expired() {
    let t = NamedTempFile::new().unwrap();
    let m = create_temp_manager(&t, 30, 4, false, "UTC");
    let (s1, _) = m.resolve_session(&SessionKey::telegram(1, None), "antigravity", "gemini-3.8-flash").await.unwrap();
    let mut u = s1.clone();
    u.set_session_id("antigravity", "sess-xyz");
    u.language = Some("ko".to_string());
    u.last_progress_msg_id = Some(777);
    let res = m.preserve_session_identity(&u).await.unwrap();
    assert_eq!(res.get_session_id("antigravity"), "sess-xyz");
    assert_eq!(res.language, Some("ko".to_string()));
    assert_eq!(res.last_progress_msg_id, Some(777));

    // Increment message_count via update_session
    let res_counted = m.update_session(&res).await.unwrap();
    assert_eq!(res_counted.provider_sessions.get("antigravity").unwrap().message_count, 1);

    // Rotate session ID to emptiness (simulating expired session_init clearance)
    let mut cleared = res_counted.clone();
    cleared.set_session_id("antigravity", "");
    cleared.last_progress_msg_id = None;
    let res2 = m.preserve_session_identity(&cleared).await.unwrap();
    assert_eq!(res2.get_session_id("antigravity"), "");
    assert_eq!(res2.get_past_session_ids("antigravity"), vec!["sess-xyz".to_string()]);
    assert_eq!(res2.last_progress_msg_id, None);
    assert_eq!(res2.provider_sessions.get("antigravity").unwrap().message_count, 0);
}

#[tokio::test]
async fn test_resolve_expired_session_rotates_and_preserves_topic_identity() {
    let t = NamedTempFile::new().unwrap();
    let m = create_temp_manager(&t, 30, 4, false, "UTC");
    let k = SessionKey::telegram(77, Some(12));
    let (mut s1, _) = m.resolve_session(&k, "antigravity", "gemini-3.8-flash").await.unwrap();
    s1.set_session_id("antigravity", "expired-sid-1");
    s1.model = "claude-opus-5-5".to_string();
    s1.language = Some("ko".to_string());
    s1.pending_attachments = vec!["telegram_files/att.pdf".to_string()];
    s1.last_handover_summary = Some("Previous handover".to_string());
    m.update_session(&s1).await.unwrap();

    // Force session expiration in storage
    let mut map = m.load().unwrap();
    let entry = map.get_mut(&k.storage_key()).unwrap();
    entry.last_active = (Utc::now() - Duration::minutes(60)).to_rfc3339();
    m.save(&map).unwrap();

    let (s2, is_new) = m.resolve_session(&k, "antigravity", "gemini-3.8-flash").await.unwrap();
    assert!(is_new);
    assert_eq!(s2.get_session_id("antigravity"), "");
    assert_eq!(s2.get_past_session_ids("antigravity"), vec!["expired-sid-1".to_string()]);
    assert_eq!(s2.model, "claude-opus-5-5");
    assert_eq!(s2.language, Some("ko".to_string()));
    assert_eq!(s2.pending_attachments, vec!["telegram_files/att.pdf".to_string()]);
    assert_eq!(s2.last_handover_summary, Some("Previous handover".to_string()));
}
