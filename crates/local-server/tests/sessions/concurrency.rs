//! 真实阻塞审计交错：取消HTTP等待不释放仍执行中的数据库工作许可；第三个请求排队而不是当场被拒
//! （BUG-017，2026-09-22：浏览器挂载页面时同时发三个读请求是常态）。

use super::{
    support::{Fixture, OWN, SESSIONS, config},
    test_support::verifier,
};
use std::{
    sync::{Arc, Condvar, Mutex},
    time::Duration,
};
use studiolo_audit_ledger::{
    AuditError, AuditEvent, AuditFilter, AuditLedger, AuditPhase, AuditRecord, SqliteAuditLedger,
};
use studiolo_auth::{OsSessionRandom, SessionManager};
use studiolo_vault_broker::ReadOnlyVault;
use tokio::sync::Semaphore;
use tower::ServiceExt;

struct Gate {
    released: Mutex<bool>,
    changed: Condvar,
    entered: Semaphore,
}

impl Gate {
    fn release(&self) {
        *self.released.lock().unwrap() = true;
        self.changed.notify_all();
    }

    async fn await_entries(&self, count: u32) {
        tokio::time::timeout(Duration::from_secs(15), self.entered.acquire_many(count))
            .await
            .unwrap()
            .unwrap()
            .forget();
    }
}

struct ReleaseOnDrop(Arc<Gate>);
impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        self.0.release();
    }
}

struct BlockingAudit {
    inner: SqliteAuditLedger,
    gate: Arc<Gate>,
}

impl AuditLedger for BlockingAudit {
    fn append(&self, event: AuditEvent) -> Result<AuditRecord, AuditError> {
        if event.action() == "session.issue" && event.phase() == AuditPhase::Intent {
            self.gate.entered.add_permits(1);
            let released = self.gate.released.lock().unwrap();
            let (released, _) = self
                .gate
                .changed
                .wait_timeout_while(released, Duration::from_secs(30), |released| !*released)
                .unwrap();
            if !*released {
                return Err(AuditError::Unavailable);
            }
        }
        self.inner.append(event)
    }

    fn query(&self, filter: &AuditFilter) -> Result<Vec<AuditRecord>, AuditError> {
        self.inner.query(filter)
    }
}

/// 两个签发请求卡在审计里、客户端都已取消：两个阻塞许可仍被真实工作占着。
macro_rules! hold_both_permits {
    () => {{
        let mut fixture = Fixture::new(true).await;
        let original = fixture.issue().await;
        let gate = Arc::new(Gate {
            released: Mutex::new(false),
            changed: Condvar::new(),
            entered: Semaphore::new(0),
        });
        let _release_on_failure = ReleaseOnDrop(gate.clone());
        let audit = Arc::new(BlockingAudit {
            inner: SqliteAuditLedger::open_existing(&fixture.audit).unwrap(),
            gate: gate.clone(),
        });
        let sessions = Arc::new(
            SessionManager::open_existing(
                &fixture.auth,
                config(),
                audit,
                fixture.clock.clone(),
                Arc::new(OsSessionRandom),
            )
            .unwrap(),
        );
        fixture.app = studiolo_local_server::router_with_sessions(
            ReadOnlyVault::open(&fixture.root.join("vault")).unwrap(),
            ReadOnlyVault::open(&fixture.root.join("web")).unwrap(),
            "虚构取消交错".into(),
            4319,
            verifier().await,
            sessions.clone(),
        )
        .unwrap();
        let first = tokio::spawn(
            fixture
                .app
                .clone()
                .oneshot(fixture.request("POST", SESSIONS, None, "{}")),
        );
        gate.await_entries(1).await;
        let second = tokio::spawn(
            fixture
                .app
                .clone()
                .oneshot(fixture.request("POST", SESSIONS, None, "{}")),
        );
        gate.await_entries(1).await;
        first.abort();
        second.abort();
        assert!(first.await.unwrap_err().is_cancelled());
        assert!(second.await.unwrap_err().is_cancelled());
        (fixture, original, gate, _release_on_failure, sessions)
    }};
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelled_requests_keep_both_blocking_slots_until_the_actual_work_finishes() {
    let (fixture, original, gate, _release_on_failure, sessions) = hold_both_permits!();

    // 两个原阻塞调用尚未返回：第三个请求排队等许可，不开第三个数据库工作，也不当场拒绝。
    let third = tokio::spawn(fixture.app.clone().oneshot(fixture.request(
        "GET",
        OWN,
        Some(&original.cookie),
        "",
    )));
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!third.is_finished());
    gate.release();

    // 真实工作一结束，许可就交给排队者：第三个请求直接成功，不需要重建Router或取消底层事务。
    let response = tokio::time::timeout(Duration::from_secs(15), third)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(response.status().as_u16(), 200);

    // 在真实任务完成后预算会恢复，不需要重建Router或取消底层业务事务。
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let response = fixture
                .app
                .clone()
                .oneshot(fixture.request("GET", OWN, Some(&original.cookie), ""))
                .await
                .unwrap();
            if response.status().as_u16() == 200 {
                break;
            }
            assert_eq!(response.status().as_u16(), 503);
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let identity = fixture.identity().await;
    let token = studiolo_auth::SessionToken::from_hex(original.token).unwrap();
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let sessions = sessions.clone();
            let identity = identity.clone();
            let token =
                studiolo_auth::SessionToken::from_hex(token.expose_secret().to_owned()).unwrap();
            let active = tokio::task::spawn_blocking(move || {
                sessions
                    .sessions(&identity, &token)
                    .unwrap()
                    .iter()
                    .filter(|row| row.state == studiolo_auth::SessionState::Active)
                    .count()
            })
            .await
            .unwrap();
            if active == 3 {
                break;
            }
            assert!((1..=3).contains(&active));
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    // 原签发即使无人等待响应也可完成；这两条孤儿将按正常期限失效，不能假称回滚。
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_request_that_cannot_get_a_permit_in_time_is_refused_as_not_executed() {
    let (fixture, original, gate, _release_on_failure, _sessions) = hold_both_permits!();

    let started = std::time::Instant::now();
    let response = fixture
        .app
        .clone()
        .oneshot(fixture.request("GET", OWN, Some(&original.cookie), ""))
        .await
        .unwrap();
    // 排满上限才拒：不是当场拒，也不会无限期挂着。
    assert!(started.elapsed() >= Duration::from_secs(4));
    assert_eq!(response.status().as_u16(), 503);
    assert!(!response.headers().contains_key("set-cookie"));
    let body = super::support::json(response).await;
    assert_eq!(body["code"], "auth_busy");
    assert!(body["error"].as_str().unwrap().contains("没有执行"));
    assert!(!body["error"].as_str().unwrap().contains("可能已生效"));
    gate.release();
}
