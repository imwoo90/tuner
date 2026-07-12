//! Unit tests for TaskRegistry persistence and CRUD operations

use std::fs;
use tempfile::TempDir;

use crate::tasks::models::TaskSubmit;
use crate::tasks::registry::TaskRegistry;

fn make_submit(name: &str) -> TaskSubmit {
    TaskSubmit {
        chat_id: 123,
        prompt: "Run tests".to_string(),
        message_id: 1,
        thread_id: None,
        parent_agent: "main".to_string(),
        name: name.to_string(),
        provider_override: "mock".to_string(),
        model_override: "mock-model".to_string(),
        thinking_override: "mock-thinking".to_string(),
        priority: "interactive".to_string(),
        depends_on: vec![],
    }
}

#[test]
fn test_creates_entry_and_folder() {
    let tmp = TempDir::new().unwrap();
    let reg_path = tmp.path().join("registry.json");
    let tasks_dir = tmp.path().join("tasks");

    let reg = TaskRegistry::new(reg_path.clone(), tasks_dir.clone()).unwrap();
    let entry = reg.create(
        make_submit("Test Task"),
        "mock".to_string(),
        "mock-model".to_string(),
        "mock-thinking".to_string(),
        None,
        None,
    ).unwrap();

    assert!(!entry.task_id.is_empty());
    assert_eq!(entry.name, "Test Task");
    assert_eq!(entry.status, "running");

    // Check directory and seed files
    let task_folder = tasks_dir.join(&entry.task_id);
    assert!(task_folder.is_dir());
    assert!(task_folder.join("TASKMEMORY.md").is_file());
    assert!(task_folder.join("CLAUDE.md").is_file());
    assert!(task_folder.join("AGENTS.md").is_file());
    assert!(task_folder.join("GEMINI.md").is_file());

    // Verify persistence
    let reloaded = TaskRegistry::new(reg_path, tasks_dir).unwrap();
    let reloaded_entry = reloaded.get(&entry.task_id).unwrap();
    // Stale recovery should have changed "running" to "failed" on reload
    assert_eq!(reloaded_entry.status, "failed");
    assert_eq!(reloaded_entry.error, "Bot restarted while task was running");
}

#[test]
fn test_auto_name_from_id() {
    let tmp = TempDir::new().unwrap();
    let reg = TaskRegistry::new(tmp.path().join("reg.json"), tmp.path().join("tasks")).unwrap();
    let mut submit = make_submit("");
    submit.name = "".to_string();

    let entry = reg.create(
        submit,
        "mock".to_string(),
        "mock-model".to_string(),
        "mock-thinking".to_string(),
        None,
        None,
    ).unwrap();

    assert_eq!(entry.name, entry.task_id);
}

#[test]
fn test_find_by_name() {
    let tmp = TempDir::new().unwrap();
    let reg = TaskRegistry::new(tmp.path().join("reg.json"), tmp.path().join("tasks")).unwrap();
    let entry = reg.create(
        make_submit("Unique Name"),
        "mock".to_string(),
        "mock-model".to_string(),
        "mock-thinking".to_string(),
        None,
        None,
    ).unwrap();

    let found = reg.find_by_name(123, "unique name").unwrap();
    assert_eq!(found.task_id, entry.task_id);

    // Wrong chat_id -> not found
    assert!(reg.find_by_name(999, "unique name").is_none());
}

#[test]
fn test_update_status() {
    let tmp = TempDir::new().unwrap();
    let reg = TaskRegistry::new(tmp.path().join("reg.json"), tmp.path().join("tasks")).unwrap();
    let entry = reg.create(
        make_submit("Update Test"),
        "mock".to_string(),
        "mock-model".to_string(),
        "mock-thinking".to_string(),
        None,
        None,
    ).unwrap();

    let ok = reg.update_status(&entry.task_id, |e| {
        e.status = "done".to_string();
        e.error = "none".to_string();
    }).unwrap();
    assert!(ok);

    let updated = reg.get(&entry.task_id).unwrap();
    assert_eq!(updated.status, "done");
    assert_eq!(updated.error, "none");
}

#[test]
fn test_list_active_and_all() {
    let tmp = TempDir::new().unwrap();
    let reg = TaskRegistry::new(tmp.path().join("reg.json"), tmp.path().join("tasks")).unwrap();
    let entry1 = reg.create(
        make_submit("T1"),
        "mock".to_string(),
        "mock-model".to_string(),
        "mock-thinking".to_string(),
        None,
        None,
    ).unwrap();

    let mut submit2 = make_submit("T2");
    submit2.chat_id = 999;
    let entry2 = reg.create(
        submit2,
        "mock".to_string(),
        "mock-model".to_string(),
        "mock-thinking".to_string(),
        None,
        None,
    ).unwrap();

    // Both are running (active)
    let active_all = reg.list_active(None);
    assert_eq!(active_all.len(), 2);

    let active_chat1 = reg.list_active(Some(123));
    assert_eq!(active_chat1.len(), 1);
    assert_eq!(active_chat1[0].task_id, entry1.task_id);

    let all = reg.list_all(None, None);
    assert_eq!(all.len(), 2);

    let all_filtered = reg.list_all(Some(999), None);
    assert_eq!(all_filtered.len(), 1);
    assert_eq!(all_filtered[0].task_id, entry2.task_id);
}

#[test]
fn test_cleanup_finished() {
    let tmp = TempDir::new().unwrap();
    let reg = TaskRegistry::new(tmp.path().join("reg.json"), tmp.path().join("tasks")).unwrap();
    let entry1 = reg.create(
        make_submit("T1"),
        "mock".to_string(),
        "mock-model".to_string(),
        "mock-thinking".to_string(),
        None,
        None,
    ).unwrap();

    let entry2 = reg.create(
        make_submit("T2"),
        "mock".to_string(),
        "mock-model".to_string(),
        "mock-thinking".to_string(),
        None,
        None,
    ).unwrap();

    // Mark T1 as done
    reg.update_status(&entry1.task_id, |e| e.status = "done".to_string()).unwrap();

    // Clean up finished -> T1 removed, T2 remains
    let cleaned = reg.cleanup_finished(None).unwrap();
    assert_eq!(cleaned, 1);

    assert!(reg.get(&entry1.task_id).is_none());
    assert!(reg.get(&entry2.task_id).is_some());

    let task1_folder = tmp.path().join("tasks").join(&entry1.task_id);
    assert!(!task1_folder.exists());
}

#[test]
fn test_delete() {
    let tmp = TempDir::new().unwrap();
    let reg = TaskRegistry::new(tmp.path().join("reg.json"), tmp.path().join("tasks")).unwrap();
    let entry = reg.create(
        make_submit("T1"),
        "mock".to_string(),
        "mock-model".to_string(),
        "mock-thinking".to_string(),
        None,
        None,
    ).unwrap();

    // Try to delete running -> fails (returns false)
    let ok = reg.delete(&entry.task_id).unwrap();
    assert!(!ok);

    // Mark done -> delete succeeds
    reg.update_status(&entry.task_id, |e| e.status = "done".to_string()).unwrap();
    let ok = reg.delete(&entry.task_id).unwrap();
    assert!(ok);
    assert!(reg.get(&entry.task_id).is_none());
}

#[test]
fn test_cleanup_orphans() {
    let tmp = TempDir::new().unwrap();
    let reg = TaskRegistry::new(tmp.path().join("reg.json"), tmp.path().join("tasks")).unwrap();
    let entry = reg.create(
        make_submit("T1"),
        "mock".to_string(),
        "mock-model".to_string(),
        "mock-thinking".to_string(),
        None,
        None,
    ).unwrap();

    // 1. Entry without folder -> should drop entry
    let folder = reg.task_folder(&entry.task_id);
    fs::remove_dir_all(&folder).unwrap();

    let cleaned = reg.cleanup_orphans().unwrap();
    assert_eq!(cleaned, 1);
    assert!(reg.get(&entry.task_id).is_none());

    // 2. Folder without entry -> should delete folder
    let orphan_folder = tmp.path().join("tasks").join("orphan_folder_123");
    fs::create_dir_all(&orphan_folder).unwrap();

    let cleaned = reg.cleanup_orphans().unwrap();
    assert_eq!(cleaned, 1);
    assert!(!orphan_folder.exists());
}
