//! # Topic History and Session Retention Tests
//!
//! ## Overview
//! Tests session rotation, historical session persistence, and `topic.json` generation.

use crate::session::data::SessionData;
use crate::messenger::telegram::history::TopicMetadata;
use tempfile::tempdir;

#[allow(clippy::too_many_arguments)]
fn log_telegram_message(
    working_dir: &std::path::Path,
    session_id: &str,
    topic_id: Option<i64>,
    topic_name: Option<&str>,
    sender: &str,
    message_id: Option<i32>,
    text: &str,
    is_success: bool,
    error: Option<&str>,
) {
    crate::messenger::telegram::history::log_telegram_message(
        working_dir,
        session_id,
        (topic_id, topic_name),
        sender,
        message_id,
        text,
        (is_success, error),
    );
}

#[test]
fn test_provider_session_past_ids_retention() {
    let mut sess = SessionData::new(100, "tg".to_string(), Some(42), "antigravity".to_string(), "gemini-3.8-flash".to_string());

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
    assert_eq!(sess.get_past_session_ids("antigravity"), vec!["sess-1".to_string(), "sess-2".to_string()]);
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

fn mock_sess(tid: i64, name: &str) -> SessionData {
    let mut s = SessionData::new(1, "tg".to_string(), Some(tid), "antigravity".to_string(), "gemini-3.8-flash".to_string());
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
}
