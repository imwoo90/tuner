use crate::bus::lock_pool::LockPool;
use std::sync::Arc;

#[test]
fn test_get_creates_lock() {
    let pool = LockPool::new(10);
    let _lock = pool.get((1, None));
    assert_eq!(pool.len(), 1);
}

#[test]
fn test_get_returns_same_lock() {
    let pool = LockPool::new(10);
    let a = pool.get((1, None));
    let b = pool.get((1, None));
    assert!(Arc::ptr_eq(&a, &b));
}

#[test]
fn test_get_with_plain_chat_helper() {
    let pool = LockPool::new(10);
    let a = pool.get_chat(42);
    let b = pool.get((42, None));
    assert!(Arc::ptr_eq(&a, &b));
}

#[test]
fn test_get_different_topics() {
    let pool = LockPool::new(10);
    let a = pool.get((1, None));
    let b = pool.get((1, Some(5)));
    assert!(!Arc::ptr_eq(&a, &b));
    assert_eq!(pool.len(), 2);
}

#[test]
fn test_is_locked_false_when_no_lock() {
    let pool = LockPool::new(10);
    assert!(!pool.is_locked((99, None)));
}

#[tokio::test]
async fn test_is_locked_true_when_held() {
    let pool = LockPool::new(10);
    let lock = pool.get((1, None));
    assert!(!pool.is_locked((1, None)));

    let _guard = lock.lock().await;
    assert!(pool.is_locked((1, None)));

    drop(_guard);
    assert!(!pool.is_locked((1, None)));
}

#[tokio::test]
async fn test_any_locked_for_chat() {
    let pool = LockPool::new(10);
    let lock = pool.get((10, Some(3)));
    assert!(!pool.any_locked_for_chat(10));

    let _guard = lock.lock().await;
    assert!(pool.any_locked_for_chat(10));
    assert!(!pool.any_locked_for_chat(99));

    drop(_guard);
    assert!(!pool.any_locked_for_chat(10));
}

#[test]
fn test_eviction_on_overflow() {
    let pool = LockPool::new(5);
    let mut locks = Vec::new();
    for i in 0..5 {
        locks.push(pool.get((i, None)));
    }
    assert_eq!(pool.len(), 5);
    let _extra = pool.get((99, None));
    assert_eq!(pool.len(), 6);
    drop(locks);
    assert_eq!(pool.len(), 1);
}

#[tokio::test]
async fn test_eviction_preserves_locked() {
    let pool = LockPool::new(3);
    let held = pool.get((1, None));
    let _extra = vec![pool.get((2, None)), pool.get((3, None))];

    let _guard = held.lock().await;
    let _extra_lock = pool.get((99, None));
    assert!(pool.is_locked((1, None)));
}

#[test]
fn test_normalize_variants() {
    let pool = LockPool::new(10);
    let a = pool.get(42);
    let b = pool.get((42, None));
    let c = pool.get((42, Some(7)));
    assert!(Arc::ptr_eq(&a, &b));
    assert!(!Arc::ptr_eq(&a, &c));
}
