//! 私有取钥 Seam 的轮换、并发和失败关闭契约。

use serde_json::json;
use std::{sync::Arc, time::Duration};
use tokio::task::JoinHandle;

use super::*;
use crate::access::test_support::{GatedSource, MemorySource, jwks};

/// 在独立任务里查找 kid，便于让它与在途取钥并发。
fn spawn_key(
    cache: &Arc<KeyCache>,
    kid: &'static str,
) -> JoinHandle<Result<DecodingKey, AccessError>> {
    let cache = Arc::clone(cache);
    tokio::spawn(async move { cache.key(kid).await })
}

/// 查找必须在极短超时内完成；暂停时钟下它若排队等锁，运行时空闲会把时钟推到超时点而失败。
async fn without_waiting(cache: &KeyCache, kid: &str) -> Result<DecodingKey, AccessError> {
    tokio::time::timeout(Duration::from_millis(10), cache.key(kid))
        .await
        .expect("查找在等待在途取钥")
}

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
    let source = GatedSource::new("test-key", "test-key");
    let cache = Arc::new(KeyCache::connect(source.clone()).await.unwrap());
    tokio::time::advance(CACHE_TTL).await;
    let task = spawn_key(&cache, "test-key");
    source.wait_for_calls(2).await;
    // 过期快照不走快路径：取钥在途时同 kid 查找只能排队等结果，拿不到旧密钥。
    assert!(
        tokio::time::timeout(Duration::from_millis(10), cache.key("test-key"))
            .await
            .is_err()
    );
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    // 冷却记录与闸门都已就绪：立即以不可用拒绝，既不重新取钥也不挂起。
    assert!(matches!(
        without_waiting(&cache, "test-key").await,
        Err(AccessError::Unavailable)
    ));
    assert_eq!(source.count(), 2);
}

#[tokio::test(start_paused = true)]
async fn cached_kid_lookup_never_waits_for_inflight_refresh() {
    let source = GatedSource::new("test-key", "new-key");
    let cache = Arc::new(KeyCache::connect(source.clone()).await.unwrap());
    tokio::time::advance(REFRESH_INTERVAL).await;
    // 未验签的任意 kid 触发刷新并卡在取钥上，此时刷新闸门被占。
    let refresh = spawn_key(&cache, "new-key");
    source.wait_for_calls(2).await;
    for _ in 0..4 {
        assert!(without_waiting(&cache, "test-key").await.is_ok());
    }
    assert!(!refresh.is_finished());
    source.release();
    assert!(refresh.await.unwrap().is_ok());
    // 刷新装入的新快照已替换旧密钥：轮换掉的 kid 在冷却内被拒绝，且不再取钥。
    assert!(matches!(
        without_waiting(&cache, "test-key").await,
        Err(AccessError::Rejected)
    ));
    assert_eq!(source.count(), 2);
}

#[tokio::test(start_paused = true)]
async fn concurrent_unknown_kids_share_one_inflight_fetch() {
    let source = GatedSource::new("test-key", "new-key");
    let cache = Arc::new(KeyCache::connect(source.clone()).await.unwrap());
    tokio::time::advance(REFRESH_INTERVAL).await;
    let first = spawn_key(&cache, "new-key");
    source.wait_for_calls(2).await;
    let same = spawn_key(&cache, "new-key");
    let other = spawn_key(&cache, "other-key");
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }
    // 后到的未知 kid 在闸门后等在途结果：既不另起取钥，也不抢先按冷却拒绝。
    assert!(!same.is_finished() && !other.is_finished());
    assert_eq!(source.count(), 2);
    source.release();
    assert!(first.await.unwrap().is_ok());
    assert!(same.await.unwrap().is_ok());
    assert!(matches!(other.await.unwrap(), Err(AccessError::Rejected)));
    assert_eq!(source.count(), 2);
}

#[tokio::test(start_paused = true)]
async fn cancelled_unknown_kid_refresh_keeps_cooldown_and_frees_gate() {
    let source = GatedSource::new("test-key", "new-key");
    let cache = Arc::new(KeyCache::connect(source.clone()).await.unwrap());
    tokio::time::advance(REFRESH_INTERVAL).await;
    let refresh = spawn_key(&cache, "new-key");
    source.wait_for_calls(2).await;
    refresh.abort();
    assert!(refresh.await.unwrap_err().is_cancelled());
    // 取消后冷却记录仍在：未知 kid 立即被拒且不取钥；已缓存 kid 照常命中。
    assert!(matches!(
        without_waiting(&cache, "new-key").await,
        Err(AccessError::Rejected)
    ));
    assert!(without_waiting(&cache, "test-key").await.is_ok());
    assert_eq!(source.count(), 2);
    // 闸门已随取消释放：冷却期满后下一次刷新照常完成。
    source.release();
    tokio::time::advance(REFRESH_INTERVAL).await;
    assert!(without_waiting(&cache, "new-key").await.is_ok());
    assert_eq!(source.count(), 3);
}
