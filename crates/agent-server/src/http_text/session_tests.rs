//! 不等待真实时间的会话生命周期与并发边界测试。
use super::*;

fn pool(total: usize, per_type: usize) -> Sessions<()> {
    let pool = Sessions::new();
    pool.inner.lock().unwrap().limits = Limits {
        total,
        per_type,
        starts_per_minute: 100,
        idle: Duration::from_secs(5),
        absolute: Duration::from_secs(10),
    };
    pool
}

#[test]
fn pending_initialization_reserves_capacity_and_drop_releases_it() {
    let pool = pool(2, 1);
    let now = Instant::now();
    let pending = pool.reserve("claude", now).unwrap();
    assert!(matches!(
        pool.reserve("claude", now),
        Err(ReserveError::Full)
    ));
    let other = pool.reserve("codex", now).unwrap();
    assert!(matches!(pool.reserve("grok", now), Err(ReserveError::Full)));
    assert!(pool.get(&pending.id, "claude", now).is_none());
    drop(pending);
    assert!(pool.reserve("claude", now).is_ok());
    drop(other);
}

#[test]
fn session_is_bound_to_type_and_rejected_lookups_do_not_renew_idle_time() {
    let pool = pool(2, 2);
    let now = Instant::now();
    let id = pool
        .reserve("claude", now)
        .unwrap()
        .commit(Arc::new(()), now)
        .unwrap();
    assert!(pool.get(&id, "codex", now).is_none());
    assert!(!pool.touch(&id, "codex", now + Duration::from_secs(4)));
    assert!(
        pool.get(&id, "claude", now + Duration::from_secs(4))
            .is_some()
    );
    assert!(
        pool.get(&id, "claude", now + Duration::from_secs(5))
            .is_none()
    );
    assert!(!pool.touch(&id, "claude", now + Duration::from_secs(5)));
    assert!(pool.get("unknown", "claude", now).is_none());
}

#[test]
fn successful_reads_extend_idle_lifetime_but_never_absolute_lifetime() {
    let pool = pool(1, 1);
    let now = Instant::now();
    let id = pool
        .reserve("claude", now)
        .unwrap()
        .commit(Arc::new(()), now)
        .unwrap();
    assert!(pool.touch(&id, "claude", now + Duration::from_secs(4)));
    assert!(pool.touch(&id, "claude", now + Duration::from_secs(8)));
    assert!(
        pool.get(&id, "claude", now + Duration::from_secs(9))
            .is_some()
    );
    assert!(
        pool.get(&id, "claude", now + Duration::from_secs(10))
            .is_none()
    );
    assert!(pool.reserve("codex", now + Duration::from_secs(10)).is_ok());
}

#[test]
fn failed_starts_are_rate_limited_even_when_reservations_are_released() {
    let pool = pool(1, 1);
    pool.inner.lock().unwrap().limits.starts_per_minute = 2;
    let now = Instant::now();
    drop(pool.reserve("claude", now).unwrap());
    drop(pool.reserve("claude", now).unwrap());
    assert!(matches!(
        pool.reserve("claude", now),
        Err(ReserveError::Full)
    ));
    assert!(
        pool.reserve("claude", now + Duration::from_secs(60))
            .is_ok()
    );
}

#[test]
fn stalled_initialization_cannot_publish_an_already_expired_session() {
    let pool = pool(1, 1);
    let now = Instant::now();
    let pending = pool.reserve("claude", now).unwrap();
    assert!(matches!(
        pending.commit(Arc::new(()), now + Duration::from_secs(10)),
        Err(ReserveError::Unavailable)
    ));
    assert!(
        pool.reserve("claude", now + Duration::from_secs(11))
            .is_ok()
    );
}

#[tokio::test]
async fn cancelling_initialization_releases_its_reserved_slot() {
    let pool = Arc::new(pool(1, 1));
    let child_pool = pool.clone();
    let (sent, received) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        let _reservation = child_pool.reserve("claude", Instant::now()).unwrap();
        sent.send(()).unwrap();
        std::future::pending::<()>().await;
    });
    received.await.unwrap();
    assert!(matches!(
        pool.reserve("claude", Instant::now()),
        Err(ReserveError::Full)
    ));
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert!(pool.reserve("claude", Instant::now()).is_ok());
}

#[test]
fn racing_starts_cannot_exceed_reserved_capacity() {
    let pool = Arc::new(pool(4, 4));
    let barrier = Arc::new(std::sync::Barrier::new(12));
    let tasks = (0..12)
        .map(|_| {
            let pool = pool.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                pool.reserve("claude", Instant::now())
            })
        })
        .collect::<Vec<_>>();
    let results = tasks
        .into_iter()
        .map(|task| task.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 4);
    drop(results);
    assert!(pool.reserve("claude", Instant::now()).is_ok());
}
