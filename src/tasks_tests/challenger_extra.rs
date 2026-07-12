//! Additional challenger tests for process tree cancellation
//!
//! Employs custom provider mock implementation to verify process tree signaling.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use tempfile::TempDir;
use async_trait::async_trait;

use crate::cli::{AgentProvider, CliResponse, StreamEvent};
use crate::tasks::{TaskHub, TasksConfig, TaskRegistry, ProcessRegistry};
use crate::tasks::models::TaskSubmit;

fn make_sub(pa: &str, n: &str) -> TaskSubmit {
    TaskSubmit {
        chat_id: 123, prompt: "t".into(), message_id: 1, thread_id: None, parent_agent: pa.into(),
        name: n.into(), provider_override: "m".into(), model_override: "m".into(),
        thinking_override: "".into(), priority: "interactive".into(), depends_on: vec![],
    }
}

fn is_process_alive(pid: u32) -> bool {
    let p = nix::unistd::Pid::from_raw(pid as i32);
    nix::sys::signal::kill(p, None).is_ok()
}

struct GrandchildLeakerProvider {
    pid: Arc<AtomicU32>,
    grandchild_pid: Arc<AtomicU32>,
    use_process_group: bool,
}

#[async_trait]
impl AgentProvider for GrandchildLeakerProvider {
    async fn send(&self, _: &str, _: Option<&str>, _: bool, _: PathBuf) -> Result<CliResponse, String> {
        let mut cmd = tokio::process::Command::new("sh");
        cmd.arg("-c")
           .arg("sleep 100 & echo $! && wait")
           .stdout(std::process::Stdio::piped())
           .stderr(std::process::Stdio::null());
        
        if self.use_process_group {
            cmd.process_group(0);
        }
        cmd.kill_on_drop(true);
        
        let mut child = cmd.spawn().map_err(|e| e.to_string())?;
        let child_pid = child.id().unwrap();
        self.pid.store(child_pid, Ordering::SeqCst);
        
        if let Some(registry) = crate::tasks::runner::GLOBAL_PROCESS_REGISTRY.get() {
            registry.register("test-leak-task".to_string(), child_pid).await;
        }
        
        let stdout = child.stdout.take().unwrap();
        let reader = tokio::io::BufReader::new(stdout);
        use tokio::io::AsyncBufReadExt;
        let mut lines = reader.lines();
        if let Some(line) = lines.next_line().await.map_err(|e| e.to_string())? {
            if let Ok(gpid) = line.trim().parse::<u32>() {
                self.grandchild_pid.store(gpid, Ordering::SeqCst);
            }
        }
        
        let _ = child.wait_with_output().await;
        
        if let Some(registry) = crate::tasks::runner::GLOBAL_PROCESS_REGISTRY.get() {
            registry.unregister("test-leak-task").await;
        }
        
        Ok(CliResponse { result: "ok".into(), session_id: None, is_error: false, returncode: Some(0), stderr: "".into() })
    }
    async fn send_streaming<'a>(&'a self, _: &str, _: Option<&str>, _: bool, _: PathBuf) -> Result<futures::stream::BoxStream<'a, StreamEvent>, String> {
        Err("".into())
    }
}

#[tokio::test]
async fn test_process_tree_leak_on_cancel() {
    let tmp = TempDir::new().unwrap();
    let reg = Arc::new(TaskRegistry::new(tmp.path().join("r.json"), tmp.path().join("t")).unwrap());
    let pid = Arc::new(AtomicU32::new(0));
    let grandchild_pid = Arc::new(AtomicU32::new(0));
    
    let prov = Arc::new(GrandchildLeakerProvider {
        pid: pid.clone(),
        grandchild_pid: grandchild_pid.clone(),
        use_process_group: false,
    });
    
    let preg = Arc::new(ProcessRegistry::new());
    let hub = Arc::new(TaskHub::new(reg.clone(), TasksConfig { enabled: true, max_parallel: 2, timeout_seconds: 3600.0 }, Some(prov), Some(preg.clone())));
    
    let mut submit = make_sub("main", "test");
    submit.name = "leak".to_string();
    let tid = hub.submit(submit).await.unwrap();
    
    let mut gpid = 0;
    let mut cpid = 0;
    for _ in 0..100 {
        gpid = grandchild_pid.load(Ordering::SeqCst);
        cpid = pid.load(Ordering::SeqCst);
        if gpid != 0 && cpid != 0 { break; }
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
    }
    assert_ne!(gpid, 0);
    assert_ne!(cpid, 0);
    
    assert!(is_process_alive(cpid));
    assert!(is_process_alive(gpid));
    
    preg.register(tid.clone(), cpid).await;
    
    assert!(hub.cancel(&tid).await);
    
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    
    let child_alive = is_process_alive(cpid);
    let grandchild_alive = is_process_alive(gpid);
    
    println!("test_process_tree_leak_on_cancel: child_alive={}, grandchild_alive={}", child_alive, grandchild_alive);
    
    if grandchild_alive {
        let _ = nix::sys::signal::kill(nix::unistd::Pid::from_raw(gpid as i32), nix::sys::signal::Signal::SIGKILL);
    }

    assert!(!child_alive, "Child should be killed");
    assert!(grandchild_alive, "Grandchild should be leaked when use_process_group is false");
}

#[tokio::test]
async fn test_process_tree_killed_with_process_group() {
    let tmp = TempDir::new().unwrap();
    let reg = Arc::new(TaskRegistry::new(tmp.path().join("r.json"), tmp.path().join("t")).unwrap());
    let pid = Arc::new(AtomicU32::new(0));
    let grandchild_pid = Arc::new(AtomicU32::new(0));
    
    let prov = Arc::new(GrandchildLeakerProvider {
        pid: pid.clone(),
        grandchild_pid: grandchild_pid.clone(),
        use_process_group: true,
    });
    
    let preg = Arc::new(ProcessRegistry::new());
    let hub = Arc::new(TaskHub::new(reg.clone(), TasksConfig { enabled: true, max_parallel: 2, timeout_seconds: 3600.0 }, Some(prov), Some(preg.clone())));
    
    let mut submit = make_sub("main", "test");
    submit.name = "group".to_string();
    let tid = hub.submit(submit).await.unwrap();
    
    let mut gpid = 0;
    let mut cpid = 0;
    for _ in 0..100 {
        gpid = grandchild_pid.load(Ordering::SeqCst);
        cpid = pid.load(Ordering::SeqCst);
        if gpid != 0 && cpid != 0 { break; }
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
    }
    assert_ne!(gpid, 0);
    assert_ne!(cpid, 0);
    
    assert!(is_process_alive(cpid));
    assert!(is_process_alive(gpid));
    
    preg.register(tid.clone(), cpid).await;
    
    assert!(hub.cancel(&tid).await);
    
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    
    let child_alive = is_process_alive(cpid);
    let grandchild_alive = is_process_alive(gpid);
    
    println!("test_process_tree_killed_with_process_group: child_alive={}, grandchild_alive={}", child_alive, grandchild_alive);
    
    assert!(!child_alive, "Child should be killed");
    assert!(!grandchild_alive, "Grandchild should be killed when process_group is set");
}

