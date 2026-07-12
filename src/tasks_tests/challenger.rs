//! Challenger tests for tasks
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use tokio::sync::Mutex;
use tempfile::TempDir;
use async_trait::async_trait;
use axum::{body::Body, http::{Request, StatusCode}, Router};
use tower::ServiceExt;
use crate::cli::{AgentProvider, CliResponse, StreamEvent};
use crate::tasks::{TaskHub, TasksConfig, TaskRegistry, TaskResult, ProcessRegistry, TaskResultCallback};
use crate::tasks::models::TaskSubmit;
use crate::webhook::api::server::ApiServerState;

struct SleepyProvider { pid: Arc<AtomicU32>, drp: Arc<std::sync::atomic::AtomicBool> }
#[async_trait]
impl AgentProvider for SleepyProvider {
    async fn send(&self, _: &str, _: Option<&str>, _: bool, _: PathBuf) -> Result<CliResponse, String> {
        struct G(Arc<std::sync::atomic::AtomicBool>);
        impl Drop for G { fn drop(&mut self) { self.0.store(true, Ordering::SeqCst); } }
        let _g = G(self.drp.clone());
        let mut cmd = tokio::process::Command::new("sleep");
        cmd.arg("10").process_group(0).kill_on_drop(true);
        let child = cmd.spawn().map_err(|e| e.to_string())?;
        self.pid.store(child.id().unwrap(), Ordering::SeqCst);
        let _ = child.wait_with_output().await;
        Ok(CliResponse { result: "ok".into(), session_id: None, is_error: false, returncode: Some(0), stderr: "".into() })
    }
    async fn send_streaming<'a>(&'a self, _: &str, _: Option<&str>, _: bool, _: PathBuf) -> Result<futures::stream::BoxStream<'a, StreamEvent>, String> {
        Err("".into())
    }
}
#[tokio::test]
async fn test_cancellation_order() {
    let tmp = TempDir::new().unwrap();
    let reg = Arc::new(TaskRegistry::new(tmp.path().join("r.json"), tmp.path().join("t")).unwrap());
    let (pid, drp) = (Arc::new(AtomicU32::new(0)), Arc::new(std::sync::atomic::AtomicBool::new(false)));
    let prov = Arc::new(SleepyProvider { pid: pid.clone(), drp: drp.clone() });
    let preg = Arc::new(ProcessRegistry::new());
    let hub = Arc::new(TaskHub::new(reg.clone(), TasksConfig { enabled: true, max_parallel: 2, timeout_seconds: 3600.0 }, Some(prov), Some(preg.clone())));
    let tid = hub.submit(make_sub("main", "test")).await.unwrap();
    let mut cpid = 0;
    for _ in 0..100 {
        cpid = pid.load(Ordering::SeqCst);
        if cpid != 0 { break; }
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
    }
    assert_ne!(cpid, 0);
    let n_pid = nix::unistd::Pid::from_raw(cpid as i32);
    assert!(nix::sys::signal::kill(n_pid, None).is_ok());
    preg.kill_for_task(&tid).await;
    assert!(nix::sys::signal::kill(n_pid, None).is_ok());
    assert!(hub.cancel(&tid).await);
    assert_eq!(reg.get(&tid).unwrap().status, "cancelled");
    assert!(drp.load(Ordering::SeqCst));
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    assert!(nix::sys::signal::kill(n_pid, None).is_err());
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
fn make_sub(pa: &str, n: &str) -> TaskSubmit {
    TaskSubmit {
        chat_id: 123, prompt: "t".into(), message_id: 1, thread_id: None, parent_agent: pa.into(),
        name: n.into(), provider_override: "m".into(), model_override: "m".into(),
        thinking_override: "".into(), priority: "interactive".into(), depends_on: vec![],
    }
}
#[tokio::test]
async fn test_isolation_and_truncation() {
    let tmp = TempDir::new().unwrap();
    let reg = Arc::new(TaskRegistry::new(tmp.path().join("r.json"), tmp.path().join("t")).unwrap());
    let (a1, a2) = (tmp.path().join("a1"), tmp.path().join("a2"));
    fs::create_dir_all(&a1).unwrap();
    fs::create_dir_all(&a2).unwrap();
    let hub = Arc::new(TaskHub::new(reg.clone(), TasksConfig { enabled: true, max_parallel: 5, timeout_seconds: 3600.0 }, Some(Arc::new(QuickProvider)), None));
    hub.set_agent_paths("agent1", a1.clone()).await;
    hub.set_agent_paths("agent2", a2.clone()).await;
    let t1 = hub.submit(make_sub("agent1", "ta")).await.unwrap();
    let t2 = hub.submit(make_sub("agent2", "tb")).await.unwrap();
    let (f1, f2) = (a1.join(&t1), a2.join(&t2));
    assert!(f1.is_dir() && f2.is_dir());
    for n in &["CLAUDE.md", "AGENTS.md", "GEMINI.md"] {
        assert!(f1.join(n).is_file() && f2.join(n).is_file());
    }
    fs::write(f1.join("TASKMEMORY.md"), &"A".repeat(4500)).unwrap();
    struct CB(Arc<Mutex<String>>);
    #[async_trait]
    impl TaskResultCallback for CB { async fn call(&self, r: TaskResult) { *self.0.lock().await = r.result_text; } }
    let text = Arc::new(Mutex::new(String::new()));
    hub.set_result_handler("agent1", Arc::new(CB(text.clone()))).await;
    for _ in 0..100 {
        if !text.lock().await.is_empty() { break; }
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
    }
    let res = text.lock().await.clone();
    assert!(res.contains("CONTENT FROM TASKMEMORY.MD") && res.contains("truncated") && res.contains(&"A".repeat(4000)));
}
#[tokio::test]
async fn test_taskmemory_utf8_panic() {
    let tmp = TempDir::new().unwrap();
    let reg = Arc::new(TaskRegistry::new(tmp.path().join("r.json"), tmp.path().join("t")).unwrap());
    let prov = Arc::new(QuickProvider);
    let hub = Arc::new(TaskHub::new(reg.clone(), TasksConfig { enabled: true, max_parallel: 5, timeout_seconds: 3600.0 }, Some(prov), None));
    let t1 = hub.submit(make_sub("agent1", "utf8")).await.unwrap();
    let mpath = tmp.path().join("t").join(&t1).join("TASKMEMORY.md");
    let mut bad = "A".repeat(3999);
    bad.push_str("✨");
    fs::write(&mpath, &bad).unwrap();
    
    let handle = hub.in_flight.write().await.get_mut(&t1).unwrap().join_handle.take().unwrap();
    let res = handle.await;
    assert!(res.is_ok());
    let entry = reg.get(&t1).unwrap();
    assert_eq!(entry.status, "done");
}
fn setup_router() -> (Router, Arc<TaskHub>, TempDir) {
    let tmp = TempDir::new().unwrap();
    let reg = Arc::new(TaskRegistry::new(tmp.path().join("r.json"), tmp.path().join("t")).unwrap());
    let prov = Arc::new(SleepyProvider { pid: Arc::new(AtomicU32::new(0)), drp: Arc::new(std::sync::atomic::AtomicBool::new(false)) });
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
        .route("/tasks/list", axum::routing::get(crate::webhook::api::tasks::handle_task_list))
        .route("/tasks/cancel", axum::routing::post(crate::webhook::api::tasks::handle_task_cancel))
        .route("/tasks/delete", axum::routing::post(crate::webhook::api::tasks::handle_task_delete))
        .with_state(state);
    (app, hub, tmp)
}
#[tokio::test]
async fn test_api_auth() {
    let (app, hub, _tmp) = setup_router();
    let tid = hub.submit(make_sub("agent1", "auth")).await.unwrap();
    let p = |u: &str, b: String, t: Option<&str>| {
        let mut r = Request::builder().method("POST").uri(u).header("content-type", "application/json");
        if let Some(tok) = t { r = r.header("Authorization", format!("Bearer {}", tok)); }
        r.body(Body::from(b)).unwrap()
    };
    let token = Some("test-token");

    assert_eq!(app.clone().oneshot(p("/tasks/cancel", format!(r#"{{"task_id":"{}","from":"agent1"}}"#, tid), None)).await.unwrap().status(), StatusCode::UNAUTHORIZED);
    assert_eq!(app.clone().oneshot(p("/tasks/cancel", format!(r#"{{"task_id":"{}","from":"agent1"}}"#, tid), Some("invalid"))).await.unwrap().status(), StatusCode::UNAUTHORIZED);
    assert_eq!(app.clone().oneshot(p("/tasks/cancel", format!(r#"{{"task_id":"{}","from":"agent2"}}"#, tid), token)).await.unwrap().status(), StatusCode::FORBIDDEN);
    assert_eq!(app.clone().oneshot(p("/tasks/resume", format!(r#"{{"task_id":"{}","prompt":"c","from":"agent2"}}"#, tid), token)).await.unwrap().status(), StatusCode::FORBIDDEN);
    assert_eq!(app.clone().oneshot(p("/tasks/cancel", format!(r#"{{"task_id":"{}","from":"agent1"}}"#, tid), token)).await.unwrap().status(), StatusCode::OK);
    assert_eq!(app.clone().oneshot(p("/tasks/delete", format!(r#"{{"task_id":"{}","from":"agent2"}}"#, tid), token)).await.unwrap().status(), StatusCode::FORBIDDEN);
    assert_eq!(app.clone().oneshot(p("/tasks/delete", format!(r#"{{"task_id":"{}","from":"agent1"}}"#, tid), token)).await.unwrap().status(), StatusCode::OK);
}

