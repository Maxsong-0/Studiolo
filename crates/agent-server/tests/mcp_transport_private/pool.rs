//! 不等待真实时钟，直接验证初始化、容量与闲置/绝对期限的交集。
use super::*;

fn limits() -> Limits {
    Limits {
        total: 2,
        per_type: 1,
        starts_per_minute: 4,
        idle: Duration::from_secs(5),
        absolute: Duration::from_secs(10),
        initialize: Duration::from_secs(2),
    }
}

#[test]
fn actual_work_is_globally_bounded_even_after_its_session_metadata_is_removed() {
    let pool = Pool::new(limits());
    let first = pool.reserve("claude", "digest-a", Instant::now()).unwrap();
    let second = pool.reserve("codex", "digest-b", Instant::now()).unwrap();
    let mut permits = (0..16)
        .map(|_| first.enter_work().unwrap())
        .collect::<Vec<_>>();
    assert!(second.enter_work().is_none());
    first.start_closing();
    pool.release(&first);
    assert!(
        second.enter_work().is_none(),
        "会话结束不会冒充供给任务已结束"
    );
    permits.pop();
    assert!(second.enter_work().is_some());
}

#[test]
fn pending_slots_count_toward_global_and_type_capacity_without_becoming_sessions() {
    let pool = Pool::new(limits());
    let now = Instant::now();
    let first = pool.reserve("claude", "digest-a", now).unwrap();
    assert!(!first.live(now));
    assert!(!first.touch(now));
    assert!(matches!(
        pool.reserve("claude", "digest-b", now),
        Err(ReserveError::Full)
    ));
    let second = pool.reserve("codex", "digest-c", now).unwrap();
    assert!(matches!(
        pool.reserve("grok", "digest-d", now),
        Err(ReserveError::Full)
    ));
    first.session.set("real-sid".into()).unwrap();
    assert_eq!(pool.find("real-sid").unwrap().owner, "claude");
    assert!(pool.find("unknown").is_none());
    pool.release(&second);
    assert!(pool.reserve("grok", "digest-d", now).is_ok());
}

#[test]
fn failed_initializations_release_capacity_but_never_refund_rate() {
    let pool = Pool::new(limits());
    let now = Instant::now();
    for _ in 0..4 {
        let binding = pool.reserve("claude", "digest", now).unwrap();
        pool.release(&binding);
    }
    assert!(matches!(
        pool.reserve("claude", "digest", now),
        Err(ReserveError::Full)
    ));
    assert!(
        pool.reserve("claude", "digest", now + Duration::from_secs(60))
            .is_ok()
    );
}

#[test]
fn expired_initialization_cannot_publish_or_free_capacity_before_worker_cleanup() {
    let pool = Pool::new(limits());
    let now = Instant::now();
    let binding = pool.reserve("claude", "digest", now).unwrap();
    let later = now + Duration::from_secs(2);
    assert!(binding.stale(later));
    assert!(!binding.initialize(later));
    assert!(matches!(
        pool.reserve("claude", "digest", later),
        Err(ReserveError::Full)
    ));
    assert!(binding.start_closing());
    assert!(!binding.start_closing());
    assert!(!binding.initialize(now));
    pool.release(&binding);
    assert!(pool.reserve("claude", "digest", later).is_ok());
}

#[test]
fn lookups_and_failed_reads_do_not_extend_idle_but_successful_reads_do() {
    let pool = Pool::new(limits());
    let now = Instant::now();
    let binding = pool.reserve("claude", "digest", now).unwrap();
    assert!(binding.initialize(now));
    assert!(!binding.initialize(now));
    assert!(binding.live(now + Duration::from_secs(4)));
    assert!(!binding.live(now + Duration::from_secs(5)));
    assert!(!binding.touch(now + Duration::from_secs(5)));
}

#[test]
fn successful_reads_never_extend_absolute_lifetime_or_revive_closing_sessions() {
    let pool = Pool::new(limits());
    let now = Instant::now();
    let binding = pool.reserve("claude", "digest", now).unwrap();
    assert!(binding.initialize(now));
    assert!(binding.touch(now + Duration::from_secs(4)));
    assert!(binding.touch(now + Duration::from_secs(8)));
    assert!(binding.live(now + Duration::from_secs(9)));
    assert!(!binding.live(now + Duration::from_secs(10)));
    assert!(!binding.touch(now + Duration::from_secs(10)));
    assert!(binding.start_closing());
    assert!(!binding.live(now));
    assert!(!binding.touch(now));
}

#[tokio::test]
async fn racing_initializations_share_a_single_atomic_capacity_budget() {
    let pool = Arc::new(Pool::new(Limits {
        total: 1,
        ..limits()
    }));
    let mut tasks = Vec::new();
    for owner in ["claude", "codex", "grok"] {
        let pool = pool.clone();
        tasks.push(tokio::spawn(async move {
            pool.reserve(owner, "digest", Instant::now()).is_ok()
        }));
    }
    let mut admitted = 0;
    for task in tasks {
        admitted += usize::from(task.await.unwrap());
    }
    assert_eq!(admitted, 1);
}
