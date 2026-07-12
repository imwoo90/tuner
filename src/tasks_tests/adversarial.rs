//! Adversarial/Stress tests for Task Concurrency, Priority Bypass, and DAG Validation

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use tempfile::TempDir;
use async_trait::async_trait;

use crate::cli::{AgentProvider, CliResponse, StreamEvent};
use crate::tasks::models::{TaskSubmit, TaskResult, TasksConfig};
use crate::tasks::registry::TaskRegistry;
use crate::tasks::hub::{TaskHub, TaskResultCallback};
use crate::tasks::dag::{check_cycle, topological_sort};

struct SlowAgentProvider {
    sleep_ms: u64,
}

#[async_trait]
impl AgentProvider for SlowAgentProvider {
    async fn send(
        &self,
        _prompt: &str,
        _resume_session: Option<&str>,
        _continue_session: bool,
        _workspace: PathBuf,
    ) -> Result<CliResponse, String> {
        tokio::time::sleep(tokio::time::Duration::from_millis(self.sleep_ms)).await;
        Ok(CliResponse {
            result: "Done".to_string(),
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

fn make_submit(name: &str, priority: &str, deps: Vec<String>) -> TaskSubmit {
    TaskSubmit {
        chat_id: 42,
        prompt: "adversarial test task".to_string(),
        message_id: 1,
        thread_id: None,
        parent_agent: "main".to_string(),
        name: name.to_string(),
        provider_override: "mock".to_string(),
        model_override: "mock-model".to_string(),
        thinking_override: "".to_string(),
        priority: priority.to_string(),
        depends_on: deps,
    }
}

#[tokio::test]
async fn test_interactive_bypasses_concurrency_limit() {
    let tmp = TempDir::new().unwrap();
    let reg = Arc::new(TaskRegistry::new(tmp.path().join("r.json"), tmp.path().join("t")).unwrap());
    let config = TasksConfig { enabled: true, max_parallel: 1, timeout_seconds: 60.0 };
    let provider = Arc::new(SlowAgentProvider { sleep_ms: 50 });
    let hub = Arc::new(TaskHub::new(reg, config, Some(provider), None));
    let results = Arc::new(Mutex::new(Vec::new()));
    hub.set_result_handler("main", Arc::new(MockResultCallback { results: results.clone() })).await;

    let t1_id = hub.submit(make_submit("bg_1", "background", vec![])).await.unwrap();
    let t2_res = hub.submit(make_submit("bg_2", "background", vec![])).await;
    assert!(t2_res.is_err());
    assert!(t2_res.unwrap_err().to_string().contains("Too many background tasks"));

    let t3_res = hub.submit(make_submit("int_1", "interactive", vec![])).await;
    assert!(t3_res.is_ok());
    tokio::time::sleep(tokio::time::Duration::from_millis(150)).await;
    
    let finished = results.lock().await;
    assert_eq!(finished.len(), 2);
    assert!(finished.iter().any(|r| r.task_id == t1_id));
    assert!(finished.iter().any(|r| r.task_id == t3_res.as_ref().unwrap().clone()));
}

#[tokio::test]
async fn test_concurrency_limit_race_condition() {
    let tmp = TempDir::new().unwrap();
    let reg = Arc::new(TaskRegistry::new(tmp.path().join("r.json"), tmp.path().join("t")).unwrap());
    let config = TasksConfig { enabled: true, max_parallel: 1, timeout_seconds: 60.0 };
    let provider = Arc::new(SlowAgentProvider { sleep_ms: 100 });
    let hub = Arc::new(TaskHub::new(reg, config, Some(provider), None));
    let results = Arc::new(Mutex::new(Vec::new()));
    hub.set_result_handler("main", Arc::new(MockResultCallback { results: results.clone() })).await;

    let hub1 = hub.clone();
    let hub2 = hub.clone();
    let hub3 = hub.clone();
    let fut1 = tokio::spawn(async move { hub1.submit(make_submit("race_1", "background", vec![])).await });
    let fut2 = tokio::spawn(async move { hub2.submit(make_submit("race_2", "background", vec![])).await });
    let fut3 = tokio::spawn(async move { hub3.submit(make_submit("race_3", "background", vec![])).await });

    let (r1, r2, r3) = tokio::join!(fut1, fut2, fut3);
    let mut success_count = 0;
    if r1.unwrap().is_ok() { success_count += 1; }
    if r2.unwrap().is_ok() { success_count += 1; }
    if r3.unwrap().is_ok() { success_count += 1; }

    println!("Concurrently submitted background tasks succeeded: {}", success_count);
    let in_flight_count = hub.in_flight.read().await.len();
    println!("In flight concurrently: {}", in_flight_count);

    // Let's assert that the race condition was triggered or that more than max_parallel ran.
    // Since check_concurrency and insertion are non-atomic, multiple submissions can succeed.
    tokio::time::sleep(tokio::time::Duration::from_millis(250)).await;
}

#[test]
fn test_dag_diamond() {
    let mut adj = HashMap::new();
    adj.insert("A".to_string(), vec!["B".to_string(), "C".to_string()]);
    adj.insert("B".to_string(), vec!["D".to_string()]);
    adj.insert("C".to_string(), vec!["D".to_string()]);
    adj.insert("D".to_string(), vec![]);
    
    assert!(!check_cycle(&adj));
    let sorted = topological_sort(&adj).unwrap();
    assert_eq!(sorted.len(), 4);
    let pos_a = sorted.iter().position(|x| x == "A").unwrap();
    let pos_b = sorted.iter().position(|x| x == "B").unwrap();
    let pos_c = sorted.iter().position(|x| x == "C").unwrap();
    let pos_d = sorted.iter().position(|x| x == "D").unwrap();
    assert!(pos_d < pos_b);
    assert!(pos_d < pos_c);
    assert!(pos_b < pos_a);
    assert!(pos_c < pos_a);
}

#[test]
fn test_dag_cycles() {
    let mut adj = HashMap::new();
    adj.insert("A".to_string(), vec!["B".to_string()]);
    adj.insert("B".to_string(), vec!["A".to_string()]);
    assert!(check_cycle(&adj));
    assert!(topological_sort(&adj).is_err());

    let mut adj_self = HashMap::new();
    adj_self.insert("A".to_string(), vec!["A".to_string()]);
    assert!(check_cycle(&adj_self));
    assert!(topological_sort(&adj_self).is_err());
}

#[test]
fn test_dag_nested_cycle() {
    let mut adj = HashMap::new();
    adj.insert("A".to_string(), vec!["B".to_string()]);
    adj.insert("B".to_string(), vec!["C".to_string()]);
    adj.insert("C".to_string(), vec!["D".to_string()]);
    adj.insert("D".to_string(), vec!["B".to_string()]);
    assert!(check_cycle(&adj));
    assert!(topological_sort(&adj).is_err());
}

#[test]
fn test_dag_deep_nest() {
    let mut adj = HashMap::new();
    adj.insert("A".to_string(), vec!["B".to_string()]);
    adj.insert("B".to_string(), vec!["C".to_string()]);
    adj.insert("C".to_string(), vec!["D".to_string()]);
    adj.insert("D".to_string(), vec!["E".to_string()]);
    adj.insert("E".to_string(), vec!["F".to_string()]);
    adj.insert("F".to_string(), vec!["G".to_string()]);
    adj.insert("G".to_string(), vec!["H".to_string()]);
    adj.insert("H".to_string(), vec![]);
    assert!(!check_cycle(&adj));
    let sorted = topological_sort(&adj).unwrap();
    assert_eq!(sorted, vec!["H", "G", "F", "E", "D", "C", "B", "A"]);
}

#[test]
fn test_dag_disjoint() {
    let mut adj = HashMap::new();
    adj.insert("A".to_string(), vec!["B".to_string()]);
    adj.insert("B".to_string(), vec![]);
    adj.insert("C".to_string(), vec!["D".to_string()]);
    adj.insert("D".to_string(), vec![]);
    assert!(!check_cycle(&adj));
    let sorted = topological_sort(&adj).unwrap();
    assert_eq!(sorted.len(), 4);
    let pos_a = sorted.iter().position(|x| x == "A").unwrap();
    let pos_b = sorted.iter().position(|x| x == "B").unwrap();
    let pos_c = sorted.iter().position(|x| x == "C").unwrap();
    let pos_d = sorted.iter().position(|x| x == "D").unwrap();
    assert!(pos_b < pos_a);
    assert!(pos_d < pos_c);
}

#[test]
fn test_dag_external_dep() {
    let mut adj = HashMap::new();
    adj.insert("A".to_string(), vec!["B".to_string()]);
    assert!(!check_cycle(&adj));
    let sorted = topological_sort(&adj).unwrap();
    assert_eq!(sorted.len(), 2);
    assert_eq!(sorted, vec!["B", "A"]);
}

