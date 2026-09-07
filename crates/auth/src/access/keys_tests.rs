//! 私有取钥 Seam 的轮换、并发和失败关闭契约。

use serde_json::json;
use std::sync::Arc;

use super::*;
use crate::access::test_support::{MemorySource, jwks};

#[tokio::test(start_paused = true)]
async fn cache_reuses_keys_then_refreshes_at_expiry() {
    let source = MemorySource::new("test-key");
    let cache = KeyCache::connect(source.clone()).await.unwrap();
    for _ in 0..5 {
        assert!(cache.key("test-key").await.is_ok());
    }
    assert_eq!(source.count(), 1);
    tokio::time::advance(CACHE_TTL).await;
    assert!(cache.key("test-key").await.is_ok());
    assert_eq!(source.count(), 2);
}

#[tokio::test(start_paused = true)]
async fn rotated_kid_refreshes_once_for_concurrent_requests() {
    let source = MemorySource::new("old-key");
    let cache = Arc::new(KeyCache::connect(source.clone()).await.unwrap());
    *source.body.lock().unwrap() = Ok(serde_json::to_vec(&jwks("new-key")).unwrap());
    assert!(matches!(
        cache.key("new-key").await,
        Err(AccessError::Rejected)
    ));
    tokio::time::advance(REFRESH_INTERVAL).await;
    let mut tasks = Vec::new();
    for _ in 0..32 {
        let cache = Arc::clone(&cache);
        tasks.push(tokio::spawn(
            async move { cache.key("new-key").await.is_ok() },
        ));
    }
    for task in tasks {
        assert!(task.await.unwrap());
    }
    assert_eq!(source.count(), 2);
    assert!(matches!(
        cache.key("old-key").await,
        Err(AccessError::Rejected)
    ));
}

#[tokio::test(start_paused = true)]
async fn different_unknown_kids_share_one_global_refresh_budget() {
    let source = MemorySource::new("test-key");
    let cache = Arc::new(KeyCache::connect(source.clone()).await.unwrap());
    tokio::time::advance(REFRESH_INTERVAL).await;
    let mut tasks = Vec::new();
    for index in 0..32 {
        let cache = Arc::clone(&cache);
        tasks.push(tokio::spawn(async move {
            cache.key(&format!("unknown-{index}")).await.is_err()
        }));
    }
    for task in tasks {
        assert!(task.await.unwrap());
    }
    assert_eq!(source.count(), 2);
    assert!(cache.key("test-key").await.is_ok());
    assert_eq!(source.count(), 2);
}

#[tokio::test(start_paused = true)]
async fn expired_keys_never_survive_failed_refresh_and_failures_are_rate_limited() {
    let source = MemorySource::new("test-key");
    let cache = KeyCache::connect(source.clone()).await.unwrap();
    *source.body.lock().unwrap() = Err(AccessError::Unavailable);
    tokio::time::advance(CACHE_TTL).await;
    for _ in 0..10 {
        assert!(matches!(
            cache.key("test-key").await,
            Err(AccessError::Unavailable)
        ));
    }
    assert_eq!(source.count(), 2);
    *source.body.lock().unwrap() = Ok(serde_json::to_vec(&jwks("test-key")).unwrap());
    assert!(cache.key("test-key").await.is_err());
    tokio::time::advance(REFRESH_INTERVAL).await;
    assert!(cache.key("test-key").await.is_ok());
    assert_eq!(source.count(), 3);
}

#[tokio::test]
async fn startup_requires_usable_keys() {
    let source = MemorySource::new("test-key");
    *source.body.lock().unwrap() = Err(AccessError::Unavailable);
    assert!(KeyCache::connect(source).await.is_err());
}

#[test]
fn jwks_is_bounded_and_rejects_duplicate_or_incompatible_keys() {
    assert!(parse(&vec![b' '; MAX_JWKS_BYTES + 1]).is_err());
    assert!(parse(b"not json").is_err());
    assert!(parse(br#"{"keys":[]}"#).is_err());
    let good = jwks("test-key");
    let duplicate = json!({"keys":[good["keys"][0].clone(), good["keys"][0].clone()]});
    assert!(parse(&serde_json::to_vec(&duplicate).unwrap()).is_err());
    let excessive = json!({"keys":vec![good["keys"][0].clone(); 9]});
    assert!(parse(&serde_json::to_vec(&excessive).unwrap()).is_err());
    for (field, value) in [
        ("kid", json!("")),
        ("kty", json!("oct")),
        ("alg", json!("HS256")),
        ("use", json!("enc")),
        ("key_ops", json!(["sign"])),
        ("n", json!("a".repeat(341))),
        ("n", json!("a".repeat(1367))),
        ("e", json!("Aw")),
    ] {
        let mut bad = good.clone();
        bad["keys"][0][field] = value;
        assert!(
            parse(&serde_json::to_vec(&bad).unwrap()).is_err(),
            "{field}"
        );
    }
}

#[tokio::test(start_paused = true)]
async fn cancelled_refresh_keeps_attempt_cooldown_and_never_returns_stale_key() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct BlockingSource(AtomicUsize);
    impl KeySource for BlockingSource {
        fn fetch(&self) -> Fetch {
            let initial = self.0.fetch_add(1, Ordering::SeqCst) == 0;
            Box::pin(async move {
                if !initial {
                    std::future::pending::<()>().await;
                }
                Ok(serde_json::to_vec(&jwks("test-key")).unwrap())
            })
        }
    }
    let source = Arc::new(BlockingSource(AtomicUsize::new(0)));
    let cache = Arc::new(KeyCache::connect(source.clone()).await.unwrap());
    tokio::time::advance(CACHE_TTL).await;
    let task_cache = Arc::clone(&cache);
    let task = tokio::spawn(async move { task_cache.key("test-key").await });
    while source.0.load(Ordering::SeqCst) != 2 {
        tokio::task::yield_now().await;
    }
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert!(matches!(
        cache.key("test-key").await,
        Err(AccessError::Unavailable)
    ));
    assert_eq!(source.0.load(Ordering::SeqCst), 2);
}
