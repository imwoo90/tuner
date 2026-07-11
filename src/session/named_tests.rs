use super::named::NamedSessionRegistry;
use tempfile::NamedTempFile;

#[test]
fn test_create_named_session_limits() {
    let t = NamedTempFile::new().unwrap();
    let r = NamedSessionRegistry::new(t.path().to_path_buf());
    
    // Create 10 sessions
    for _ in 0..10 {
        let s = r.create_session(42, "idle", "").unwrap();
        assert!(!s.name.is_empty());
    }
    
    // The 11th should fail
    let err = r.create_session(42, "idle", "");
    assert!(err.is_err());
}

#[test]
fn test_crash_recovery() {
    let t = NamedTempFile::new().unwrap();
    let r = NamedSessionRegistry::new(t.path().to_path_buf());
    
    // Write JSON file directly to simulate crash on startup state
    let json = r#"{
        "boldowl": {
            "name": "boldowl",
            "chat_id": 42,
            "status": "running",
            "last_prompt": "hello world"
        },
        "ia-swiftfox": {
            "name": "ia-swiftfox",
            "chat_id": 42,
            "status": "running",
            "last_prompt": "inter-agent prompt"
        },
        "calmbear": {
            "name": "calmbear",
            "chat_id": 42,
            "status": "idle",
            "last_prompt": "idle prompt"
        }
    }"#;
    std::fs::write(t.path(), json).unwrap();
    
    r.recover_crash().unwrap();
    
    // Reload and assert
    let s1 = r.get_session("boldowl").unwrap().unwrap();
    assert_eq!(s1.status, "idle");
    assert_eq!(s1.last_prompt, "hello world");
    
    let s2 = r.get_session("ia-swiftfox").unwrap().unwrap();
    assert_eq!(s2.status, "running"); // Keep running since it starts with "ia-"
    
    let s3 = r.get_session("calmbear").unwrap().unwrap();
    assert_eq!(s3.status, "idle");
}

#[test]
fn test_truncate_last_prompt() {
    let t = NamedTempFile::new().unwrap();
    let r = NamedSessionRegistry::new(t.path().to_path_buf());
    
    let long_prompt = "a".repeat(5000);
    let s = r.create_session(42, "idle", &long_prompt).unwrap();
    assert_eq!(s.last_prompt.len(), 5000); // Not truncated for idle creation
    
    // Update to running
    let s2 = r.update_session_status(&s.name, "running", None).unwrap();
    assert_eq!(s2.last_prompt.len(), 4000);
    
    // Create running directly
    let s3 = r.create_session(42, "running", &long_prompt).unwrap();
    assert_eq!(s3.last_prompt.len(), 4000);
}
