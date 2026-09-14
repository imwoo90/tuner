//! # Topic History and Session Retention Tests
//!
//! ## Overview
//! Tests session rotation, historical session persistence, topic.json generation,
//! and topic-based session querying.

use crate::session::data::SessionData;
use crate::messenger::telegram::history::{log_telegram_message, find_sessions_for_topic, TopicMetadata};
use tempfile::tempdir;

#[test]
fn test_provider_session_past_ids_retention() {
    let mut sess = SessionData::new(100, "tg".to_string(), Some(42), "antigravity".to_string(), "gemini".to_string());
    
    // First session
    sess.set_session_id("antigravity", "sess-1");
    assert_eq!(sess.get_session_id("antigravity"), "sess-1");
    assert!(sess.get_past_session_ids("antigravity").is_empty());

    // Same session should not append to past_session_ids
    sess.set_session_id("antigravity", "sess-1");
    assert!(sess.get_past_session_ids("antigravity").is_empty());

    // Rotate to session 2
    sess.set_session_id("antigravity", "sess-2");
    assert_eq!(sess.get_session_id("antigravity"), "sess-2");
    assert_eq!(sess.get_past_session_ids("antigravity"), vec!["sess-1".to_string()]);

    // Clear session
    sess.clear_provider_session("antigravity");
    assert_eq!(sess.get_session_id("antigravity"), "");
    assert_eq!(sess.get_past_session_ids("antigravity"), vec!["sess-1".to_string(), "sess-2".to_string()]);

    // Set new session 3
    sess.set_session_id("antigravity", "sess-3");
    assert_eq!(sess.get_session_id("antigravity"), "sess-3");
    assert_eq!(sess.get_all_session_ids("antigravity"), vec!["sess-1".to_string(), "sess-2".to_string(), "sess-3".to_string()]);
}

#[test]
fn test_log_telegram_message_and_topic_json() {
    let tmp = tempdir().unwrap();
    let ws = tmp.path();

    log_telegram_message(
        ws,
        "session-alpha",
        Some(77),
        Some("Project X"),
        "user",
        Some(101),
        "Hello from user",
        true,
        None,
    );

    let topic_file = ws.join("brain").join("session-alpha").join("topic.json");
    assert!(topic_file.exists());
    let content = std::fs::read_to_string(&topic_file).unwrap();
    let meta: TopicMetadata = serde_json::from_str(&content).unwrap();
    assert_eq!(meta.session_id, "session-alpha");
    assert_eq!(meta.topic_id, Some(77));
    assert_eq!(meta.topic_name, Some("Project X".to_string()));

    let history_file = ws.join("brain").join("session-alpha").join("telegram_history.jsonl");
    assert!(history_file.exists());
    let h_content = std::fs::read_to_string(&history_file).unwrap();
    assert!(h_content.contains("\"topic_id\":77"));
    assert!(h_content.contains("\"topic_name\":\"Project X\""));
}

#[test]
fn test_find_sessions_for_topic_query() {
    let tmp = tempdir().unwrap();
    let ws = tmp.path();

    log_telegram_message(ws, "s-1", Some(77), Some("A"), "user", Some(1), "msg 1", true, None);
    log_telegram_message(ws, "s-2", Some(77), Some("A"), "user", Some(2), "msg 2", true, None);
    log_telegram_message(ws, "s-3", Some(88), Some("B"), "user", Some(3), "msg 3", true, None);

    let found_77 = find_sessions_for_topic(ws, 77);
    assert_eq!(found_77.len(), 2);
    assert!(found_77.contains(&"s-1".to_string()));
    assert!(found_77.contains(&"s-2".to_string()));

    let found_88 = find_sessions_for_topic(ws, 88);
    assert_eq!(found_88, vec!["s-3".to_string()]);

    assert!(find_sessions_for_topic(ws, 999).is_empty());
}

fn mock_sess(tid: i64, name: &str) -> SessionData {
    let mut s = SessionData::new(1, "tg".to_string(), Some(tid), "antigravity".to_string(), "flash".to_string());
    s.topic_name = Some(name.to_string());
    s
}

#[test]
fn test_multi_topic_session_lifecycle_simulation() {
    let tmp = tempdir().unwrap();
    let ws = tmp.path();

    let mut sess_a = mock_sess(10, "Feature A");
    sess_a.set_session_id("antigravity", "sess-a1");
    log_telegram_message(ws, "sess-a1", Some(10), Some("Feature A"), "user", Some(1), "Topic A msg 1", true, None);

    let mut sess_b = mock_sess(20, "Feature B");
    sess_b.set_session_id("antigravity", "sess-b1");
    log_telegram_message(ws, "sess-b1", Some(20), Some("Feature B"), "user", Some(2), "Topic B msg 1", true, None);

    sess_a.set_session_id("antigravity", "sess-a2");
    log_telegram_message(ws, "sess-a2", Some(10), Some("Feature A"), "user", Some(3), "Topic A msg 2", true, None);

    sess_b.set_session_id("antigravity", "sess-b2");
    log_telegram_message(ws, "sess-b2", Some(20), Some("Feature B"), "user", Some(4), "Topic B msg 2", true, None);

    assert_eq!(sess_a.get_past_session_ids("antigravity"), vec!["sess-a1".to_string()]);
    assert_eq!(sess_a.get_session_id("antigravity"), "sess-a2");
    assert_eq!(sess_b.get_past_session_ids("antigravity"), vec!["sess-b1".to_string()]);
    assert_eq!(sess_b.get_session_id("antigravity"), "sess-b2");

    let sessions_a = find_sessions_for_topic(ws, 10);
    assert_eq!(sessions_a.len(), 2);
    assert!(sessions_a.contains(&"sess-a1".to_string()) && sessions_a.contains(&"sess-a2".to_string()));

    let sessions_b = find_sessions_for_topic(ws, 20);
    assert_eq!(sessions_b.len(), 2);
    assert!(sessions_b.contains(&"sess-b1".to_string()) && sessions_b.contains(&"sess-b2".to_string()));
}
