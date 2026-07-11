use crate::bus::envelope::{Envelope, Origin, LockMode, DeliveryMode};
use crate::bus::adapters::{
    from_interagent_result, from_task_result, from_task_question, from_user_message,
    InterAgentResult, TaskResult,
};

fn fake_interagent_result() -> InterAgentResult {
    InterAgentResult {
        task_id: "ia1".to_string(),
        sender: "agent-a".to_string(),
        recipient: "agent-b".to_string(),
        message_preview: "please do X".to_string(),
        result_text: "X is done".to_string(),
        success: true,
        error: None,
        elapsed_seconds: 2.0,
        session_name: "ia-agent-a".to_string(),
        provider_switch_notice: String::new(),
        original_message: "full message".to_string(),
        chat_id: 0,
        topic_id: None,
    }
}

fn fake_task_result() -> TaskResult {
    TaskResult {
        task_id: "t1".to_string(),
        chat_id: 100,
        parent_agent: "main".to_string(),
        name: "research".to_string(),
        prompt_preview: "find info".to_string(),
        result_text: "found it".to_string(),
        status: "done".to_string(),
        elapsed_seconds: 5.0,
        provider: "claude".to_string(),
        model: "sonnet".to_string(),
        session_id: "tsid1".to_string(),
        error: String::new(),
        task_folder: "/tmp/tasks/t1".to_string(),
        original_prompt: "find info about X".to_string(),
        thread_id: None,
    }
}

#[test]
fn test_from_interagent_success_without_prompt_no_injection() {
    let env = from_interagent_result(&fake_interagent_result(), 100, None, None);
    assert_eq!(env.origin, Origin::Interagent);
    assert_eq!(env.chat_id, 100);
    assert_eq!(env.status, "success");
    assert_eq!(env.delivery, DeliveryMode::Unicast);
    assert_eq!(env.lock_mode, LockMode::Required);
    assert!(!env.needs_injection);
    assert_eq!(env.prompt, "");
    assert_eq!(env.metadata.get("sender").map(|s| s.as_str()), Some("agent-a"));
}

#[test]
fn test_from_interagent_success_with_prompt_enables_injection() {
    let prompt = "[ASYNC INTER-AGENT RESPONSE from 'dev' (task t1)]\nresult\n[END]";
    let env = from_interagent_result(&fake_interagent_result(), 100, Some(prompt), None);
    assert_eq!(env.origin, Origin::Interagent);
    assert!(env.needs_injection);
    assert_eq!(env.prompt, prompt);
    assert_eq!(env.lock_mode, LockMode::Required);
}

#[test]
fn test_from_interagent_matrix_transport() {
    let prompt = "[ASYNC INTER-AGENT RESPONSE from 'dev' (task t1)]\nresult\n[END]";
    let env = from_interagent_result(&fake_interagent_result(), 100, Some(prompt), Some("mx"));
    assert_eq!(env.transport, "mx");
    assert_eq!(env.metadata.get("transport").map(|s| s.as_str()), Some("mx"));
}

#[test]
fn test_from_interagent_error() {
    let mut res = fake_interagent_result();
    res.success = false;
    res.error = Some("timeout".to_string());
    let env = from_interagent_result(&res, 100, None, None);
    assert_eq!(env.status, "error");
    assert!(env.is_error);
    assert_eq!(env.lock_mode, LockMode::None);
    assert!(!env.needs_injection);
    assert_eq!(env.metadata.get("error").map(|s| s.as_str()), Some("timeout"));
}

#[test]
fn test_from_interagent_result_uses_result_chat_id() {
    let mut res = fake_interagent_result();
    res.chat_id = 777;
    res.topic_id = Some(42);
    let env = from_interagent_result(&res, 100, None, None);
    assert_eq!(env.chat_id, 777);
    assert_eq!(env.topic_id, Some(42));
}

#[test]
fn test_from_interagent_result_falls_back_to_default_chat_id() {
    let mut res = fake_interagent_result();
    res.chat_id = 0;
    res.topic_id = None;
    let env = from_interagent_result(&res, 100, None, None);
    assert_eq!(env.chat_id, 100);
    assert!(env.topic_id.is_none());
}

#[test]
fn test_from_interagent_error_preserves_topic_id() {
    let mut res = fake_interagent_result();
    res.success = false;
    res.error = Some("fail".to_string());
    res.chat_id = 555;
    res.topic_id = Some(99);
    let env = from_interagent_result(&res, 100, None, None);
    assert_eq!(env.chat_id, 555);
    assert_eq!(env.topic_id, Some(99));
    assert!(env.is_error);
}

#[test]
fn test_from_task_result_done() {
    let env = from_task_result(&fake_task_result());
    assert_eq!(env.origin, Origin::TaskResult);
    assert_eq!(env.chat_id, 100);
    assert!(env.topic_id.is_none());
    assert_eq!(env.status, "done");
    assert_eq!(env.lock_mode, LockMode::Required);
    assert!(env.needs_injection);
    assert!(!env.is_error);
    assert_eq!(env.metadata.get("name").map(|s| s.as_str()), Some("research"));
    assert!(env.prompt.contains("BACKGROUND TASK COMPLETED"));
    assert!(env.prompt.contains("task_id='t1'"));
    assert!(env.prompt.contains("found it"));
    assert!(env.prompt.contains("Review this result critically"));
}

#[test]
fn test_from_task_result_with_topic() {
    let mut res = fake_task_result();
    res.thread_id = Some(42);
    let env = from_task_result(&res);
    assert_eq!(env.chat_id, 100);
    assert_eq!(env.topic_id, Some(42));
}

#[test]
fn test_from_task_result_failed() {
    let mut res = fake_task_result();
    res.status = "failed".to_string();
    res.error = "crash".to_string();
    let env = from_task_result(&res);
    assert_eq!(env.lock_mode, LockMode::Required);
    assert!(env.needs_injection);
    assert!(env.is_error);
    assert_eq!(env.metadata.get("error").map(|s| s.as_str()), Some("crash"));
    assert!(env.prompt.contains("BACKGROUND TASK FAILED"));
    assert!(env.prompt.contains("crash"));
}

#[test]
fn test_from_task_result_cancelled() {
    let mut res = fake_task_result();
    res.status = "cancelled".to_string();
    let env = from_task_result(&res);
    assert_eq!(env.lock_mode, LockMode::None);
    assert!(!env.needs_injection);
}

#[test]
fn test_from_task_result_preserves_parent_agent() {
    let mut res = fake_task_result();
    res.parent_agent = "sonic".to_string();
    let env = from_task_result(&res);
    assert_eq!(env.metadata.get("parent_agent").map(|s| s.as_str()), Some("sonic"));
}

#[test]
fn test_from_task_question() {
    let env = from_task_question("t1", "what color?", "what co...", 100, None);
    assert_eq!(env.origin, Origin::TaskQuestion);
    assert_eq!(env.chat_id, 100);
    assert!(env.topic_id.is_none());
    assert_eq!(env.prompt, "what color?");
    assert_eq!(env.lock_mode, LockMode::Required);
    assert!(env.needs_injection);
    assert_eq!(env.metadata.get("task_id").map(|s| s.as_str()), Some("t1"));
}

#[test]
fn test_from_task_question_with_topic() {
    let env = from_task_question("t1", "what color?", "what co...", 100, Some(42));
    assert_eq!(env.chat_id, 100);
    assert_eq!(env.topic_id, Some(42));
}

#[test]
fn test_from_user_message_default_origin() {
    let env = from_user_message(100, "hello world", None, None);
    assert_eq!(env.origin, Origin::User);
    assert_eq!(env.chat_id, 100);
    assert_eq!(env.prompt, "hello world");
    assert_eq!(env.prompt_preview, "hello world");
    assert_eq!(env.delivery, DeliveryMode::Unicast);
    assert_eq!(env.lock_mode, LockMode::None);
    assert!(env.topic_id.is_none());
}

#[test]
fn test_from_user_message_api_source() {
    let env = from_user_message(200, "api request", None, Some(Origin::Api));
    assert_eq!(env.origin, Origin::Api);
    assert_eq!(env.chat_id, 200);
    assert_eq!(env.prompt, "api request");
}

#[test]
fn test_from_user_message_with_topic() {
    let env = from_user_message(300, "topic msg", Some(42), None);
    assert_eq!(env.chat_id, 300);
    assert_eq!(env.topic_id, Some(42));
}

#[test]
fn test_from_user_message_truncates_preview() {
    let long_text = "x".repeat(200);
    let env = from_user_message(100, &long_text, None, None);
    assert_eq!(env.prompt_preview.len(), 80);
    assert_eq!(env.prompt, long_text);
}

#[test]
fn test_from_user_message_empty_text() {
    let env = from_user_message(100, "", None, None);
    assert_eq!(env.prompt, "");
    assert_eq!(env.prompt_preview, "");
}
