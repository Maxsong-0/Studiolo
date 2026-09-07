//! 真实写故障与激活提交前检查失败，不冒称 COMMIT I/O 或断电实验。

use super::*;
use std::sync::{Arc, atomic::Ordering};
use studiolo_audit_ledger::AuditPhase;

#[test]
fn pending_insert_and_outbox_ack_write_failures_never_activate() {
    for phase in [AuditPhase::Intent, AuditPhase::Outcome] {
        let h = Harness::new(10, 100);
        let weak = Arc::downgrade(&h.manager);
        *h.audit.hook.lock().unwrap() = Some(Arc::new(move |event| {
            if event.action() == "session.issue" && event.phase() == phase {
                weak.upgrade()
                    .unwrap()
                    .database
                    .connection
                    .lock()
                    .unwrap()
                    .execute_batch("PRAGMA query_only=ON")
                    .unwrap();
            }
        }));
        assert_eq!(
            h.manager.issue(&identity()).unwrap_err(),
            SessionError::StorageUnavailable
        );
        assert_eq!(
            scalar(
                &h.manager,
                "SELECT count(*) FROM sessions WHERE state='active'"
            ),
            0
        );
        *h.audit.hook.lock().unwrap() = None;
        h.manager
            .database
            .connection
            .lock()
            .unwrap()
            .execute_batch("PRAGMA query_only=OFF")
            .unwrap();
        h.manager.flush_audit(256).unwrap();
        assert_eq!(
            scalar(
                &h.manager,
                "SELECT count(*) FROM sessions WHERE state='active'"
            ),
            0
        );
    }
}

#[test]
fn final_activation_storage_check_failure_preserves_ack_but_never_grants() {
    use std::{os::unix::fs::PermissionsExt, path::PathBuf, sync::atomic::AtomicBool};
    struct PermissionClock {
        after_outcome: AtomicBool,
        database: PathBuf,
    }
    impl SessionClock for PermissionClock {
        fn now_unix_ms(&self) -> Result<i64, SessionError> {
            if self.after_outcome.swap(false, Ordering::SeqCst) {
                std::fs::set_permissions(&self.database, std::fs::Permissions::from_mode(0o644))
                    .unwrap();
            }
            Ok(1_000_000)
        }
    }
    let directory = directory();
    let root = directory.path().canonicalize().unwrap();
    let clock = Arc::new(PermissionClock {
        after_outcome: AtomicBool::new(false),
        database: root.join("auth.sqlite3"),
    });
    let audit = Arc::new(FaultAudit::default());
    let weak_clock = Arc::downgrade(&clock);
    // 审计回执确认后的首次时间采样就是激活检查，不依赖内部预检调用次数。
    *audit.hook.lock().unwrap() = Some(Arc::new(move |event| {
        if event.action() == "session.issue" && event.phase() == AuditPhase::Outcome {
            weak_clock
                .upgrade()
                .unwrap()
                .after_outcome
                .store(true, Ordering::SeqCst);
        }
    }));
    let random = Arc::new(TestRandom::default());
    let manager = SessionManager::open(
        &root,
        config(10, 100),
        audit.clone(),
        clock.clone(),
        random.clone(),
    )
    .unwrap();
    assert_eq!(
        manager.issue(&identity()).unwrap_err(),
        SessionError::UnsafeStorage
    );
    assert_eq!(
        scalar(
            &manager,
            "SELECT count(*) FROM sessions WHERE grant_receipt IS NOT NULL"
        ),
        1
    );
    assert_eq!(
        scalar(
            &manager,
            "SELECT count(*) FROM sessions WHERE state='active'"
        ),
        0
    );
    std::fs::set_permissions(
        root.join("auth.sqlite3"),
        std::fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    drop(manager);
    let reopened = SessionManager::open(&root, config(10, 100), audit, clock, random).unwrap();
    reopened.flush_audit(256).unwrap();
    assert_eq!(
        scalar(
            &reopened,
            "SELECT count(*) FROM sessions WHERE state='active'"
        ),
        0
    );
}

#[test]
fn real_sqlite_busy_rejects_check_then_recovers() {
    let h = Harness::new(10, 100);
    let issued = h.manager.issue(&identity()).unwrap();
    let connection = rusqlite::Connection::open(h.root.join("auth.sqlite3")).unwrap();
    connection.execute_batch("BEGIN IMMEDIATE").unwrap();
    assert_eq!(
        h.manager.check(&identity(), &issued.token).unwrap_err(),
        SessionError::StorageUnavailable
    );
    connection.execute_batch("ROLLBACK").unwrap();
    h.manager.check(&identity(), &issued.token).unwrap();
}

#[test]
fn real_sqlite_full_never_returns_an_uncommitted_session() {
    let h = Harness::new(10, 100);
    h.manager.issue(&identity()).unwrap();
    {
        let connection = h.manager.database.connection.lock().unwrap();
        let pages: i64 = connection
            .query_row("PRAGMA page_count", [], |r| r.get(0))
            .unwrap();
        let _: i64 = connection
            .query_row(&format!("PRAGMA max_page_count={pages}"), [], |r| r.get(0))
            .unwrap();
        let failure = connection
            .execute("UPDATE auth_meta SET config=?1", ["x".repeat(256 * 1024)])
            .unwrap_err();
        assert!(
            matches!(failure,rusqlite::Error::SqliteFailure(error,_) if error.code==rusqlite::ErrorCode::DiskFull)
        );
    }
    let mut failed = false;
    for _ in 0..64 {
        let before = scalar(
            &h.manager,
            "SELECT count(*) FROM sessions WHERE state='active'",
        );
        if let Err(error) = h.manager.issue(&identity()) {
            assert_eq!(error, SessionError::StorageUnavailable);
            assert_eq!(
                scalar(
                    &h.manager,
                    "SELECT count(*) FROM sessions WHERE state='active'"
                ),
                before
            );
            failed = true;
            break;
        }
    }
    assert!(failed);
    let _: i64 = h
        .manager
        .database
        .connection
        .lock()
        .unwrap()
        .query_row("PRAGMA max_page_count=2147483646", [], |r| r.get(0))
        .unwrap();
    h.audit.fail.store(false, Ordering::SeqCst);
    h.manager.flush_audit(256).unwrap();
    h.manager.issue(&identity()).unwrap();
}
