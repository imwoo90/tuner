//! Additional challenger tests for UTF-8 slicing and HTTP API auth
//!
//! Employs custom provider mock implementation to verify edge cases of
//! UTF-8 multi-byte slicing boundaries and complete API authentication.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use tempfile::TempDir;
use async_trait::async_trait;
use axum::{body::Body, http::{Request, StatusCode}, Router};
use tower::ServiceExt;

use crate::cli::{AgentProvider, CliResponse, StreamEvent};
use crate::tasks::{TaskHub, TasksConfig, TaskRegistry, TaskResult, TaskResultCallback};
use crate::tasks::models::TaskSubmit;
use crate::webhook::api::server::ApiServerState;

fn make_sub(pa: &str, n: &str) -> TaskSubmit {
    TaskSubmit {
        chat_id: 123, prompt: "t".into(), message_id: 1, thread_id: None, parent_agent: pa.into(),
        name: n.into(), provider_override: "m".into(), model_override: "m".into(),
        thinking_override: "".into(), priority: "interactive".into(), depends_on: vec![],
    }
}

struct QuickProvider;
#[async_trait]
impl AgentProvider for QuickProvider {
    async fn send(&self, _: &str, _: Option<&str>, _: bool, _: PathBuf) -> Result<CliResponse, String> {
        Ok(CliResponse { result: "ok".into(), session_id: None, is_error: false, returncode: Some(0), stderr: "".into() })
    }
    async fn send_streaming<'a>(&'a self, _: &str, _: Option<&str>, _: bool, _: PathBuf) -> Result<futures::stream::BoxStream<'a, StreamEvent>, String> {
        Err("".into())
    }
}

#[tokio::test]
async fn test_taskmemory_utf8_slicing_mismatch() {
    let tmp = TempDir::new().unwrap();
    let reg = Arc::new(TaskRegistry::new(tmp.path().join("r.json"), tmp.path().join("t")).unwrap());
    let prov = Arc::new(QuickProvider);
    let hub = Arc::new(TaskHub::new(reg.clone(), TasksConfig { enabled: true, max_parallel: 5, timeout_seconds: 3600.0 }, Some(prov), None));
    
    struct CB(Arc<Mutex<String>>);
    #[async_trait]
    impl TaskResultCallback for CB { async fn call(&self, r: TaskResult) { *self.0.lock().await = r.result_text; } }
    let text = Arc::new(Mutex::new(String::new()));
    hub.set_result_handler("agent1", Arc::new(CB(text.clone()))).await;
    
    let t1 = hub.submit(make_sub("agent1", "utf8_slice")).await.unwrap();
    let mpath = tmp.path().join("t").join(&t1).join("TASKMEMORY.md");
    
    let content = "✨".repeat(1500);
    fs::write(&mpath, &content).unwrap();
    
    let handle = hub.in_flight.write().await.get_mut(&t1).unwrap().join_handle.take().unwrap();
    let _ = handle.await;
    
    let result_text = text.lock().await.clone();
    
    assert!(!result_text.contains("truncated"), "Should NOT contain the truncated message because character count 1500 <= 4000");
    assert!(result_text.contains(&content), "Should contain the ENTIRE content");

    // Verify that a string > 4000 characters DOES trigger truncation
    *text.lock().await = String::new();
    let t2 = hub.submit(make_sub("agent1", "utf8_slice_2")).await.unwrap();
    let mpath2 = tmp.path().join("t").join(&t2).join("TASKMEMORY.md");
    let content_long = "✨".repeat(4005);
    fs::write(&mpath2, &content_long).unwrap();

    let handle2 = hub.in_flight.write().await.get_mut(&t2).unwrap().join_handle.take().unwrap();
    let _ = handle2.await;

    let result_text2 = text.lock().await.clone();
    assert!(result_text2.contains("truncated"), "Should contain the truncated message because character count 4005 > 4000");
}

fn setup_auth_test_app() -> (Router, String, TempDir) {
    let tmp = TempDir::new().unwrap();
    let reg = Arc::new(TaskRegistry::new(tmp.path().join("r.json"), tmp.path().join("t")).unwrap());
    let prov = Arc::new(QuickProvider);
    let hub = Arc::new(TaskHub::new(reg, TasksConfig { enabled: true, max_parallel: 5, timeout_seconds: 3600.0 }, Some(prov), None));
    let state = Arc::new(std::sync::Mutex::new(ApiServerState {
        config: crate::config::ApiConfig { host: "127.0.0.1".to_string(), port: 8080, token: "test-token".to_string(), ..Default::default() },
        default_chat_id: 123, lock_pool: Arc::new(crate::bus::lock_pool::LockPool::new_default()),
        message_handler: None, abort_handler: None, allowed_roots: None, upload_dir: None, workspace: None,
        provider_info: serde_json::json!([]), active_state_getter: None, next_conn_id: 0,
        active_ws: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())), task_hub: Some(hub.clone()),
    }));
    let app = Router::new()
        .route("/tasks/create", axum::routing::post(crate::webhook::api::tasks::handle_task_create))
        .route("/tasks/resume", axum::routing::post(crate::webhook::api::tasks::handle_task_resume))
        .route("/tasks/ask_parent", axum::routing::post(crate::webhook::api::tasks::handle_task_ask_parent))
        .route("/tasks/list", axum::routing::get(crate::webhook::api::tasks::handle_task_list))
        .route("/tasks/cancel", axum::routing::post(crate::webhook::api::tasks::handle_task_cancel))
        .route("/tasks/delete", axum::routing::post(crate::webhook::api::tasks::handle_task_delete))
        .with_state(state);
        
    let tid = futures::executor::block_on(hub.submit(make_sub("agent1", "auth_test"))).unwrap();
    (app, tid, tmp)
}

fn build_req(method: &str, uri: &str, body: String, token: Option<&str>) -> Request<Body> {
    let mut r = Request::builder().method(method).uri(uri).header("content-type", "application/json");
    if let Some(t) = token { r = r.header("Authorization", format!("Bearer {}", t)); }
    r.body(Body::from(body)).unwrap()
}

#[tokio::test]
async fn test_api_auth_create_unauthorized() {
    let (app, _, _tmp) = setup_auth_test_app();
    let r1 = app.clone().oneshot(build_req("POST", "/tasks/create", r#"{"from":"agent1","prompt":"hello","name":"t"}"#.to_string(), None)).await.unwrap();
    assert_eq!(r1.status(), StatusCode::UNAUTHORIZED);
    let r2 = app.clone().oneshot(build_req("POST", "/tasks/create", r#"{"from":"agent1","prompt":"hello","name":"t"}"#.to_string(), Some("bad"))).await.unwrap();
    assert_eq!(r2.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_api_auth_resume_unauthorized() {
    let (app, tid, _tmp) = setup_auth_test_app();
    let r1 = app.clone().oneshot(build_req("POST", "/tasks/resume", format!(r#"{{"task_id":"{}","prompt":"c","from":"agent1"}}"#, tid), None)).await.unwrap();
    assert_eq!(r1.status(), StatusCode::UNAUTHORIZED);
    let r2 = app.clone().oneshot(build_req("POST", "/tasks/resume", format!(r#"{{"task_id":"{}","prompt":"c","from":"agent1"}}"#, tid), Some("bad"))).await.unwrap();
    assert_eq!(r2.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_api_auth_ask_parent_unauthorized() {
    let (app, tid, _tmp) = setup_auth_test_app();
    let r1 = app.clone().oneshot(build_req("POST", "/tasks/ask_parent", format!(r#"{{"task_id":"{}","question":"q"}}"#, tid), None)).await.unwrap();
    assert_eq!(r1.status(), StatusCode::UNAUTHORIZED);
    let r2 = app.clone().oneshot(build_req("POST", "/tasks/ask_parent", format!(r#"{{"task_id":"{}","question":"q"}}"#, tid), Some("bad"))).await.unwrap();
    assert_eq!(r2.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_api_auth_list_unauthorized() {
    let (app, _, _tmp) = setup_auth_test_app();
    let r1 = app.clone().oneshot(build_req("GET", "/tasks/list?from=agent1", String::new(), None)).await.unwrap();
    assert_eq!(r1.status(), StatusCode::UNAUTHORIZED);
    let r2 = app.clone().oneshot(build_req("GET", "/tasks/list?from=agent1", String::new(), Some("bad"))).await.unwrap();
    assert_eq!(r2.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_api_auth_cancel_unauthorized() {
    let (app, tid, _tmp) = setup_auth_test_app();
    let r1 = app.clone().oneshot(build_req("POST", "/tasks/cancel", format!(r#"{{"task_id":"{}","from":"agent1"}}"#, tid), None)).await.unwrap();
    assert_eq!(r1.status(), StatusCode::UNAUTHORIZED);
    let r2 = app.clone().oneshot(build_req("POST", "/tasks/cancel", format!(r#"{{"task_id":"{}","from":"agent1"}}"#, tid), Some("bad"))).await.unwrap();
    assert_eq!(r2.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_api_auth_delete_unauthorized() {
    let (app, tid, _tmp) = setup_auth_test_app();
    let r1 = app.clone().oneshot(build_req("POST", "/tasks/delete", format!(r#"{{"task_id":"{}","from":"agent1"}}"#, tid), None)).await.unwrap();
    assert_eq!(r1.status(), StatusCode::UNAUTHORIZED);
    let r2 = app.clone().oneshot(build_req("POST", "/tasks/delete", format!(r#"{{"task_id":"{}","from":"agent1"}}"#, tid), Some("bad"))).await.unwrap();
    assert_eq!(r2.status(), StatusCode::UNAUTHORIZED);
}
