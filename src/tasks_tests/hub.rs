//! Tests for TaskHub coordination, execution, resume, cancel, and concurrency limits

use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use tempfile::TempDir;
use async_trait::async_trait;

use crate::cli::{AgentProvider, CliResponse, StreamEvent};
use crate::tasks::models::{TaskSubmit, TaskResult, TasksConfig};
use crate::tasks::registry::TaskRegistry;
use crate::tasks::hub::{TaskHub, TaskResultCallback, QuestionHandler};
use crate::tasks::runner::ProcessRegistry;

struct MockAgentProvider {
    result_text: String,
    session_id: Option<String>,
    is_error: bool,
    returncode: Option<i32>,
}

#[async_trait]
impl AgentProvider for MockAgentProvider {
    async fn send(
        &self,
        _prompt: &str,
        _resume_session: Option<&str>,
        _continue_session: bool,
        _workspace: PathBuf,
    ) -> Result<CliResponse, String> {
        if self.is_error {
            return Err("mock error".to_string());
        }
        Ok(CliResponse {
            result: self.result_text.clone(),
            session_id: self.session_id.clone(),
            is_error: false,
            returncode: self.returncode,
            stderr: "".to_string(),
        })
    }

    async fn send_streaming<'a>(
        &'a self,
        _prompt: &str,
        _resume_session: Option<&str>,
        _continue_session: bool,
        _workspace: PathBuf,
    ) -> Result<futures::stream::BoxStream<'a, StreamEvent>, String> {
        Err("not implemented".to_string())
    }
}

struct BlockingProvider;
#[async_trait]
impl AgentProvider for BlockingProvider {
    async fn send(
        &self,
        _prompt: &str,
        _resume_session: Option<&str>,
        _continue_session: bool,
        _workspace: PathBuf,
    ) -> Result<CliResponse, String> {
        tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;
        Err("should have been cancelled".to_string())
    }
    async fn send_streaming<'a>(
        &'a self,
        _prompt: &str,
        _resume_session: Option<&str>,
        _continue_session: bool,
        _workspace: PathBuf,
    ) -> Result<futures::stream::BoxStream<'a, StreamEvent>, String> {
        Err("not implemented".to_string())
    }
}

struct MockResultCallback {
    results: Arc<Mutex<Vec<TaskResult>>>,
}

#[async_trait]
impl TaskResultCallback for MockResultCallback {
    async fn call(&self, result: TaskResult) {
        self.results.lock().await.push(result);
    }
}

struct MockQuestionHandler {
    answer: String,
}

#[async_trait]
impl QuestionHandler for MockQuestionHandler {
    async fn call(
        &self,
        _task_id: &str,
        _question: &str,
        _prompt_preview: &str,
        _chat_id: i64,
        _thread_id: Option<i64>,
    ) -> Result<String, String> {
        Ok(self.answer.clone())
    }
}

fn make_submit(name: &str, priority: &str) -> TaskSubmit {
    TaskSubmit {
        chat_id: 1,
        prompt: "task prompt".to_string(),
        message_id: 0,
        thread_id: None,
        parent_agent: "main".to_string(),
        name: name.to_string(),
        provider_override: "mock".to_string(),
        model_override: "mock-model".to_string(),
        thinking_override: "".to_string(),
        priority: priority.to_string(),
        depends_on: vec![],
    }
}

#[tokio::test]
async fn test_hub_submit_and_run() {
    let tmp = TempDir::new().unwrap();
    let registry = Arc::new(TaskRegistry::new(tmp.path().join("reg.json"), tmp.path().join("tasks")).unwrap());

    let provider = Arc::new(MockAgentProvider {
        result_text: "Task completed successfully".to_string(),
        session_id: Some("session-123".to_string()),
        is_error: false,
        returncode: Some(0),
    });

    let config = TasksConfig { enabled: true, max_parallel: 2, timeout_seconds: 3600.0 };
    let hub = Arc::new(TaskHub::new(registry, config, Some(provider), None));

    let results = Arc::new(Mutex::new(Vec::new()));
    hub.set_result_handler("main", Arc::new(MockResultCallback { results: results.clone() })).await;

    let task_id = hub.submit(make_submit("Task1", "interactive")).await.unwrap();
    assert!(!task_id.is_empty());

    let mut success = false;
    for _ in 0..100 {
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        let guard = results.lock().await;
        if !guard.is_empty() {
            success = true;
            assert_eq!(guard[0].task_id, task_id);
            assert_eq!(guard[0].status, "done");
            assert_eq!(guard[0].session_id, "session-123");
            break;
        }
    }
    assert!(success);
}

#[tokio::test]
async fn test_hub_concurrency_cap() {
    let tmp = TempDir::new().unwrap();
    let registry = Arc::new(TaskRegistry::new(tmp.path().join("reg.json"), tmp.path().join("tasks")).unwrap());

    let provider = Arc::new(MockAgentProvider {
        result_text: "ok".to_string(),
        session_id: None,
        is_error: false,
        returncode: Some(0),
    });

    let config = TasksConfig { enabled: true, max_parallel: 1, timeout_seconds: 3600.0 };
    let hub = Arc::new(TaskHub::new(registry, config, Some(provider), None));

    let t1 = hub.submit(make_submit("T1", "batch")).await.unwrap();

    let t2_res = hub.submit(make_submit("T2", "batch")).await;
    assert!(t2_res.is_err());
    assert!(t2_res.unwrap_err().to_string().contains("Too many background tasks"));

    let t3 = hub.submit(make_submit("T3", "interactive")).await;
    assert!(t3.is_ok());

    hub.cancel(&t1).await;
}

#[tokio::test]
async fn test_hub_disabled() {
    let tmp = TempDir::new().unwrap();
    let registry = Arc::new(TaskRegistry::new(tmp.path().join("reg.json"), tmp.path().join("tasks")).unwrap());
    let config = TasksConfig { enabled: false, max_parallel: 2, timeout_seconds: 3600.0 };
    let hub = Arc::new(TaskHub::new(registry, config, None, None));

    let res = hub.submit(make_submit("Task1", "interactive")).await;
    assert!(res.is_err());
    assert_eq!(res.unwrap_err().to_string(), "Task system is disabled");
}

#[tokio::test]
async fn test_hub_cancel_running() {
    let tmp = TempDir::new().unwrap();
    let registry = Arc::new(TaskRegistry::new(tmp.path().join("reg.json"), tmp.path().join("tasks")).unwrap());

    let results = Arc::new(Mutex::new(Vec::new()));
    let proc_reg = Arc::new(ProcessRegistry::new());
    let hub = Arc::new(TaskHub::new(registry, TasksConfig { enabled: true, max_parallel: 2, timeout_seconds: 3600.0 }, Some(Arc::new(BlockingProvider)), Some(proc_reg)));
    hub.set_result_handler("main", Arc::new(MockResultCallback { results: results.clone() })).await;

    let task_id = hub.submit(make_submit("BlockingTask", "interactive")).await.unwrap();
    tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;

    let cancelled = hub.cancel(&task_id).await;
    assert!(cancelled);

    let mut success = false;
    for _ in 0..100 {
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        let guard = results.lock().await;
        if !guard.is_empty() {
            success = true;
            assert_eq!(guard[0].task_id, task_id);
            assert_eq!(guard[0].status, "cancelled");
            break;
        }
    }
    assert!(success);
}
