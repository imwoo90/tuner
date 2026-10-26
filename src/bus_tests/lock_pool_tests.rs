#![allow(clippy::unwrap_used, clippy::expect_used)]
use crate::bus::lock_pool::LockPool;
use std::sync::Arc;

#[test]
fn test_get_returns_same_lock_while_held() {
    let pool = LockPool::new();
    let a = pool.get((1, None));
    let b = pool.get((1, None));
    assert!(Arc::ptr_eq(&a, &b));
}

#[test]
fn test_get_different_topics_returns_distinct_locks() {
    let pool = LockPool::new();
    let a = pool.get((1, None));
    let b = pool.get((1, Some(5)));
    assert!(!Arc::ptr_eq(&a, &b));
}

#[tokio::test]
async fn test_lock_mutual_exclusion() {
    let pool = LockPool::new();
    let lock = pool.get((1, None));
    assert!(lock.try_lock().is_ok());

    let _guard = lock.lock().await;
    assert!(pool.get((1, None)).try_lock().is_err());

    drop(_guard);
    assert!(pool.get((1, None)).try_lock().is_ok());
}

#[test]
fn test_weak_ref_pruning_recreates_after_drop() {
    let pool = LockPool::new();
    let weak = {
        let strong = pool.get((1, None));
        Arc::downgrade(&strong)
    };
    assert!(weak.upgrade().is_none());

    let fresh = pool.get((1, None));
    assert!(Arc::strong_count(&fresh) == 1);
}
