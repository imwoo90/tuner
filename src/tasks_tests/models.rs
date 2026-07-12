//! Unit tests for task models and priority normalization

use crate::tasks::models::{TaskEntry, TaskSubmit, TaskResult, normalise_priority};

#[test]
fn test_normalise_priority() {
    assert_eq!(normalise_priority(None), "background");
    assert_eq!(normalise_priority(Some("interactive")), "interactive");
    assert_eq!(normalise_priority(Some("batch")), "batch");
    assert_eq!(normalise_priority(Some("background")), "background");
    assert_eq!(normalise_priority(Some("unknown")), "background");
}

#[test]
fn test_task_entry_serialization_defaults() {
    let json_str = r#"{
        "task_id": "abc",
        "chat_id": 42,
        "parent_agent": "main",
        "name": "Task Name",
        "prompt_preview": "Prompt...",
        "provider": "claude",
        "model": "opus",
        "status": "running",
        "created_at": 1000.0,
        "original_prompt": "Prompt full content",
        "thinking": "high",
        "tasks_dir": "/tmp",
        "priority": "interactive",
        "depends_on": []
    }"#;

    let entry: TaskEntry = serde_json::from_str(json_str).unwrap();
    assert_eq!(entry.task_id, "abc");
    assert_eq!(entry.chat_id, 42);
    assert_eq!(entry.parent_agent, "main");
    assert_eq!(entry.name, "Task Name");
    assert_eq!(entry.provider, "claude");
    assert_eq!(entry.model, "opus");
    assert_eq!(entry.status, "running");
    assert_eq!(entry.session_id, "");
    assert_eq!(entry.created_at, 1000.0);
    assert_eq!(entry.completed_at, 0.0);
    assert_eq!(entry.elapsed_seconds, 0.0);
    assert_eq!(entry.error, "");
    assert_eq!(entry.result_preview, "");
    assert_eq!(entry.question_count, 0);
    assert_eq!(entry.num_turns, 0);
    assert_eq!(entry.last_question, "");
    assert_eq!(entry.original_prompt, "Prompt full content");
    assert_eq!(entry.thinking, "high");
    assert_eq!(entry.tasks_dir, "/tmp");
    assert_eq!(entry.thread_id, None);
    assert_eq!(entry.priority, "interactive");
    assert_eq!(entry.depends_on.len(), 0);
}

#[test]
fn test_task_submit_fields() {
    let submit = TaskSubmit {
        chat_id: 123,
        prompt: "Hello".to_string(),
        message_id: 456,
        thread_id: Some(789),
        parent_agent: "agent".to_string(),
        name: "Test".to_string(),
        provider_override: "openai".to_string(),
        model_override: "gpt-4".to_string(),
        thinking_override: "medium".to_string(),
        priority: "background".to_string(),
        depends_on: vec!["parent_1".to_string()],
    };

    assert_eq!(submit.chat_id, 123);
    assert_eq!(submit.prompt, "Hello");
    assert_eq!(submit.message_id, 456);
    assert_eq!(submit.thread_id, Some(789));
    assert_eq!(submit.parent_agent, "agent");
    assert_eq!(submit.name, "Test");
    assert_eq!(submit.provider_override, "openai");
    assert_eq!(submit.model_override, "gpt-4");
    assert_eq!(submit.thinking_override, "medium");
    assert_eq!(submit.priority, "background");
    assert_eq!(submit.depends_on, vec!["parent_1".to_string()]);
}

#[test]
fn test_task_result_fields() {
    let res = TaskResult {
        task_id: "t1".to_string(),
        chat_id: 1,
        parent_agent: "a1".to_string(),
        name: "n1".to_string(),
        prompt_preview: "p1".to_string(),
        result_text: "r1".to_string(),
        status: "done".to_string(),
        elapsed_seconds: 1.5,
        provider: "pr1".to_string(),
        model: "m1".to_string(),
        session_id: "s1".to_string(),
        error: "e1".to_string(),
        task_folder: "f1".to_string(),
        original_prompt: "op1".to_string(),
        thread_id: Some(999),
    };

    assert_eq!(res.task_id, "t1");
    assert_eq!(res.chat_id, 1);
    assert_eq!(res.parent_agent, "a1");
    assert_eq!(res.name, "n1");
    assert_eq!(res.result_text, "r1");
    assert_eq!(res.status, "done");
    assert_eq!(res.elapsed_seconds, 1.5);
    assert_eq!(res.thread_id, Some(999));
}
