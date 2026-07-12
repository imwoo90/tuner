//! Integration tests for Axum Task API endpoints

use axum::{
    body::Body,
    http::{Request, StatusCode},
    Router,
};
use std::sync::Arc;
use tempfile::TempDir;
use tower::ServiceExt;
use async_trait::async_trait;

use crate::cli::{AgentProvider, CliResponse, StreamEvent};
use crate::webhook::api::server::ApiServerState;
use crate::tasks::{TaskHub, TasksConfig, TaskRegistry};

struct MockAgentProvider;
#[async_trait]
impl AgentProvider for MockAgentProvider {
    async fn send(
        &self,
        _prompt: &str,
        _resume_session: Option<&str>,
        _continue_session: bool,
        _workspace: std::path::PathBuf,
    ) -> Result<CliResponse, String> {
        Ok(CliResponse {
            result: "ok".to_string(),
            session_id: Some("session-1".to_string()),
            is_error: false,
            returncode: Some(0),
            stderr: "".to_string(),
        })
    }
    async fn send_streaming<'a>(
        &'a self,
        _prompt: &str,
        _resume_session: Option<&str>,
        _continue_session: bool,
        _workspace: std::path::PathBuf,
    ) -> Result<futures::stream::BoxStream<'a, StreamEvent>, String> {
        Err("not implemented".to_string())
    }
}

fn setup_test_router() -> (Router, Arc<TaskHub>, TempDir) {
    let tmp = TempDir::new().unwrap();
    let registry = Arc::new(TaskRegistry::new(tmp.path().join("reg.json"), tmp.path().join("tasks")).unwrap());
    let provider = Arc::new(MockAgentProvider);
    let hub = Arc::new(TaskHub::new(registry, TasksConfig { enabled: true, max_parallel: 2, timeout_seconds: 3600.0 }, Some(provider), None));

    let state = Arc::new(std::sync::Mutex::new(ApiServerState {
        config: crate::config::ApiConfig {
            host: "127.0.0.1".to_string(),
            port: 8080,
            token: "test-token".to_string(),
            ..Default::default()
        },
        default_chat_id: 123,
        lock_pool: Arc::new(crate::bus::lock_pool::LockPool::new_default()),
        message_handler: None,
        abort_handler: None,
        allowed_roots: None,
        upload_dir: None,
        workspace: None,
        provider_info: serde_json::json!([]),
        active_state_getter: None,
        next_conn_id: 0,
        active_ws: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
        task_hub: Some(hub.clone()),
    }));

    let app = Router::new()
        .route("/tasks/create", axum::routing::post(crate::webhook::api::tasks::handle_task_create))
        .route("/tasks/resume", axum::routing::post(crate::webhook::api::tasks::handle_task_resume))
        .route("/tasks/list", axum::routing::get(crate::webhook::api::tasks::handle_task_list))
        .route("/tasks/cancel", axum::routing::post(crate::webhook::api::tasks::handle_task_cancel))
        .route("/tasks/delete", axum::routing::post(crate::webhook::api::tasks::handle_task_delete))
        .with_state(state);

    (app, hub, tmp)
}

#[tokio::test]
async fn test_api_create_task() {
    let (app, _, _tmp) = setup_test_router();

    let req = Request::builder()
        .method("POST")
        .uri("/tasks/create")
        .header("content-type", "application/json")
        .header("Authorization", "Bearer test-token")
        .body(Body::from(
            r#"{"from": "main", "prompt": "Hello Task", "name": "Task1", "priority": "interactive"}"#,
        ))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body_bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let body_str = String::from_utf8(body_bytes.to_vec()).unwrap();
    assert!(body_str.contains(r#""success":true"#));
    assert!(body_str.contains(r#""task_id":"#));
}

#[tokio::test]
async fn test_api_create_missing_prompt() {
    let (app, _, _tmp) = setup_test_router();

    let req = Request::builder()
        .method("POST")
        .uri("/tasks/create")
        .header("content-type", "application/json")
        .header("Authorization", "Bearer test-token")
        .body(Body::from(
            r#"{"from": "main", "prompt": "", "name": "Task1"}"#,
        ))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    let body_bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let body_str = String::from_utf8(body_bytes.to_vec()).unwrap();
    assert!(body_str.contains(r#""success":false"#));
    assert!(body_str.contains(r#""error":"Missing prompt""#));
}

#[tokio::test]
async fn test_api_list_tasks() {
    let (app, hub, _tmp) = setup_test_router();

    let submit = crate::tasks::models::TaskSubmit {
        chat_id: 123,
        prompt: "Run".to_string(),
        message_id: 1,
        thread_id: None,
        parent_agent: "main".to_string(),
        name: "TaskName".to_string(),
        provider_override: "mock".to_string(),
        model_override: "mock-model".to_string(),
        thinking_override: "".to_string(),
        priority: "interactive".to_string(),
        depends_on: vec![],
    };
    hub.submit(submit).await.unwrap();

    let req = Request::builder()
        .method("GET")
        .uri("/tasks/list?from=main")
        .header("Authorization", "Bearer test-token")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body_bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let body_str = String::from_utf8(body_bytes.to_vec()).unwrap();
    assert!(body_str.contains(r#""tasks":["#));
    assert!(body_str.contains(r#""name":"TaskName""#));
}

#[tokio::test]
async fn test_api_cancel_unauthorized() {
    let (app, hub, _tmp) = setup_test_router();

    let submit = crate::tasks::models::TaskSubmit {
        chat_id: 123,
        prompt: "Run".to_string(),
        message_id: 1,
        thread_id: None,
        parent_agent: "main".to_string(),
        name: "TaskName".to_string(),
        provider_override: "mock".to_string(),
        model_override: "mock-model".to_string(),
        thinking_override: "".to_string(),
        priority: "interactive".to_string(),
        depends_on: vec![],
    };
    let task_id = hub.submit(submit).await.unwrap();

    let req = Request::builder()
        .method("POST")
        .uri("/tasks/cancel")
        .header("content-type", "application/json")
        .header("Authorization", "Bearer test-token")
        .body(Body::from(format!(
            r#"{{"task_id": "{}", "from": "other"}}"#,
            task_id
        )))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}
