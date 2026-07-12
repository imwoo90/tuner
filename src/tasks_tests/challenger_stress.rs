//! Stress tests for background task system
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;
use async_trait::async_trait;
use crate::cli::{AgentProvider, CliResponse, StreamEvent};
use crate::tasks::{TaskHub, TasksConfig, TaskRegistry, TaskResult, TaskResultCallback};
use crate::tasks::models::TaskSubmit;

fn make_sub(pa: &str, n: &str) -> TaskSubmit {
    TaskSubmit {
        chat_id: 123, prompt: "t".into(), message_id: 1, thread_id: None, parent_agent: pa.into(),
        name: n.into(), provider_override: "m".into(), model_override: "m".into(),
        thinking_override: "".into(), priority: "interactive".into(), depends_on: vec![],
    }
}

struct PanickingProvider;
#[async_trait]
impl AgentProvider for PanickingProvider {
    async fn send(&self, _: &str, _: Option<&str>, _: bool, _: PathBuf) -> Result<CliResponse, String> {
        panic!("simulated provider panic");
    }
    async fn send_streaming<'a>(&'a self, _: &str, _: Option<&str>, _: bool, _: PathBuf) -> Result<futures::stream::BoxStream<'a, StreamEvent>, String> {
        Err("".into())
    }
}

struct TestResultCallback {
    results: Arc<tokio::sync::Mutex<Vec<TaskResult>>>,
}
#[async_trait]
impl TaskResultCallback for TestResultCallback {
    async fn call(&self, result: TaskResult) {
        self.results.lock().await.push(result);
    }
}

#[tokio::test]
async fn test_panic_task_leak_cleanup() {
    let tmp = TempDir::new().unwrap();
    let reg = Arc::new(TaskRegistry::new(tmp.path().join("r.json"), tmp.path().join("t")).unwrap());
    let prov = Arc::new(PanickingProvider);
    let hub = Arc::new(TaskHub::new(reg.clone(), TasksConfig { enabled: true, max_parallel: 2, timeout_seconds: 3600.0 }, Some(prov), None));
    let results = Arc::new(tokio::sync::Mutex::new(Vec::new()));
    hub.set_result_handler("main", Arc::new(TestResultCallback { results: results.clone() })).await;
    let mut submit = make_sub("main", "panic_test");
    submit.priority = "background".to_string();
    let tid = hub.submit(submit).await.unwrap();
    let mut success = false;
    for _ in 0..100 {
        tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
        let entry = reg.get(&tid).unwrap();
        if entry.status == "failed" {
            success = true;
            assert_eq!(entry.error, "Task execution panicked or aborted");
            break;
        }
    }
    assert!(success);
    let inflight = hub.in_flight.read().await;
    assert!(!inflight.contains_key(&tid));
    let guard = results.lock().await;
    assert_eq!(guard.len(), 1);
    assert_eq!(guard[0].status, "failed");
}

struct SlowMockProvider { sleep_ms: u64 }
#[async_trait]
impl AgentProvider for SlowMockProvider {
    async fn send(&self, _: &str, _: Option<&str>, _: bool, _: PathBuf) -> Result<CliResponse, String> {
        tokio::time::sleep(tokio::time::Duration::from_millis(self.sleep_ms)).await;
        Ok(CliResponse { result: "ok".into(), session_id: Some("session-abc".into()), is_error: false, returncode: Some(0), stderr: "".into() })
    }
    async fn send_streaming<'a>(&'a self, _: &str, _: Option<&str>, _: bool, _: PathBuf) -> Result<futures::stream::BoxStream<'a, StreamEvent>, String> {
        Err("".into())
    }
}

#[tokio::test]
async fn test_concurrency_bypass_via_resume() {
    let tmp = TempDir::new().unwrap();
    let reg = Arc::new(TaskRegistry::new(tmp.path().join("r.json"), tmp.path().join("t")).unwrap());
    let prov = Arc::new(SlowMockProvider { sleep_ms: 100 });
    let hub = Arc::new(TaskHub::new(reg.clone(), TasksConfig { enabled: true, max_parallel: 1, timeout_seconds: 60.0 }, Some(prov), None));
    let mut s1 = make_sub("main", "t1");
    s1.priority = "background".to_string();
    let t1 = hub.submit(s1).await.unwrap();
    let mut s2 = make_sub("main", "t2");
    s2.priority = "background".to_string();
    assert!(hub.submit(s2).await.is_err());
    let mut sr = make_sub("main", "res");
    sr.priority = "background".to_string();
    let entry = reg.create(sr, "m".to_string(), "m".to_string(), "".to_string(), None, None).unwrap();
    let rid = entry.task_id.clone();
    reg.update_status(&rid, |e| {
        e.status = "done".to_string();
        e.session_id = "s-res".to_string();
        e.provider = "m".to_string();
    }).unwrap();
    // Resuming now should fail because max_parallel is 1 and t1 is still running
    assert!(hub.resume(&rid, "f").await.is_err());

    // Wait for t1 to finish
    tokio::time::sleep(tokio::time::Duration::from_millis(150)).await;

    // Now resumption should succeed
    assert!(hub.resume(&rid, "f").await.is_ok());
    let inf = hub.in_flight.read().await;
    assert_eq!(inf.len(), 1);
    assert!(inf.contains_key(&rid));
}
