//! Intent 错误与未交付授权回执的拒绝计数；不以损坏回执或补记重试获得会话。

use super::*;
use rusqlite::params;
use sha2::{Digest, Sha256};
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicI64, Ordering},
    },
};
use studiolo_audit_ledger::{
    AuditError, AuditEvent, AuditEventId, AuditFilter, AuditLedger, AuditPhase, AuditRecord,
    IntegrityHash,
};

#[derive(Clone, Copy, Debug)]
enum Fault {
    Unavailable,
    Mismatch,
    DamagedHash,
}

struct ReceiptAudit {
    ledger: Arc<FaultAudit>,
    fault: Fault,
}

impl AuditLedger for ReceiptAudit {
    fn append(&self, event: AuditEvent) -> Result<AuditRecord, AuditError> {
        if event.action() != "session.issue" || event.phase() != AuditPhase::Intent {
            return self.ledger.append(event);
        }
        let event = if matches!(self.fault, Fault::Mismatch) {
            AuditEvent::new(
                AuditEventId::parse("different-intent").unwrap(),
                event.phase(),
                event.action(),
                event.payload().clone(),
            )
        } else {
            event
        };
        // FaultAudit 未开启其它故障：这里使用其真实 MemoryAuditLedger 回执。
        let record = self.ledger.append(event)?;
        record.verify_integrity().unwrap();
        if matches!(self.fault, Fault::DamagedHash) {
            let damaged = IntegrityHash::sha256(b"damaged-intent-receipt");
            assert_ne!(damaged, record.record_hash());
            Ok(AuditRecord::from_parts(
                record.offset(),
                record.recorded_at(),
                record.previous_hash(),
                damaged,
                record.event().clone(),
            ))
        } else {
            Ok(record)
        }
    }

    fn query(&self, filter: &AuditFilter) -> Result<Vec<AuditRecord>, AuditError> {
        self.ledger.query(filter)
    }
}

fn open(root: &Path, audit: Arc<ReceiptAudit>) -> SessionManager {
    SessionManager::open(
        root,
        config(10, 100),
        audit,
        Arc::new(TestClock(AtomicI64::new(1_000_000))),
        Arc::new(TestRandom::default()),
    )
    .unwrap()
}

fn assert_no_session_and_count(manager: &SessionManager, expected: i64) {
    assert_eq!(scalar(manager, "SELECT count(*) FROM sessions"), 0);
    assert_eq!(
        scalar(manager, "SELECT total FROM auth_denials WHERE kind=6"),
        expected
    );
    assert_eq!(
        scalar(manager, "SELECT sum(total) FROM auth_denials"),
        expected
    );
}

#[test]
fn intent_append_mismatch_and_damaged_hash_count_once_or_fail_closed_on_storage_error() {
    for fault in [Fault::Unavailable, Fault::Mismatch, Fault::DamagedHash] {
        for writable in [true, false] {
            let directory = directory();
            let root = directory.path().canonicalize().unwrap();
            let ledger = Arc::new(FaultAudit::default());
            ledger
                .fail_intent
                .store(matches!(fault, Fault::Unavailable), Ordering::SeqCst);
            let audit = Arc::new(ReceiptAudit {
                ledger: ledger.clone(),
                fault,
            });
            let manager = Arc::new(open(&root, audit.clone()));
            if !writable {
                let weak = Arc::downgrade(&manager);
                *ledger.hook.lock().unwrap() = Some(Arc::new(move |_| {
                    weak.upgrade()
                        .unwrap()
                        .database
                        .connection
                        .lock()
                        .unwrap()
                        .execute_batch("PRAGMA query_only=ON")
                        .unwrap();
                }));
            }
            let expected = if writable {
                SessionError::AuditUnavailable
            } else {
                SessionError::StorageUnavailable
            };
            assert_eq!(
                manager.issue(&identity()).unwrap_err(),
                expected,
                "{fault:?}"
            );
            let count = i64::from(writable);
            assert_no_session_and_count(&manager, count);
            *ledger.hook.lock().unwrap() = None;
            manager
                .database
                .connection
                .lock()
                .unwrap()
                .execute_batch("PRAGMA query_only=OFF")
                .unwrap();
            drop(manager);
            let reopened = open(&root, audit);
            assert_no_session_and_count(&reopened, count);
            assert_eq!(
                scalar(
                    &reopened,
                    "SELECT count(*) FROM audit_outbox WHERE bucket=6"
                ),
                count
            );
        }
    }
}

fn seed_bounded_backlog(manager: &SessionManager) {
    manager.database.transaction(|db| {
        let namespace = crate::session::schema::store_id(db)?;
        // 单事务生成86个规范已撤销夹具，共258条待补记，避免1024轮签发。
        for index in 1_u64..=86 {
            let issuance=crate::session::credential_store::reserve(db)?;
            let id = SessionId::parse(format!("v2-{issuance:016x}")).unwrap();
            let grant = crate::session::events::Grant { store_id: namespace.clone(), id: id.clone(),
                epoch: 1, created: 1_000_000, absolute: 1_100_000, access_expiry: 4_102_444_800_000 };
            let event = grant.event(AuditPhase::Outcome)?;
            let token = Sha256::digest(format!("backlog-token-{index}"));
            let csrf = Sha256::digest(format!("backlog-csrf-{index}"));
            db.execute("INSERT INTO sessions(id,token_digest,csrf_digest,epoch,state,created,last_seen,absolute_expiry,access_expiry,grant_id) VALUES(?1,?2,?3,1,'revoked',1000000,1000000,1100000,4102444800000,?4)",
                params![id.as_str(),token.as_slice(),csrf.as_slice(),event.event_id().as_str()]).unwrap();
            crate::session::credential_store::insert(db,&id,issuance,&token.into(),&csrf.into())?;
            crate::session::outbox::enqueue(db, &event, "issue", None)?;
            for phase in [AuditPhase::Intent, AuditPhase::Outcome] {
                let event = crate::session::events::event(&format!("{namespace}:revoke:{}", id.as_str()),
                    phase, "session.revoked", serde_json::json!({"scope":"one","target":id.as_str(),
                        "revoked":1,"identity_version":1,"effective":"already_revoked"}))?;
                crate::session::outbox::enqueue(db, &event, "revoke", None)?;
            }
        }
        Ok(())
    }).unwrap();
}

#[test]
fn missing_grant_receipt_after_bounded_flush_counts_once_and_never_recovers_activation() {
    let harness = Harness::new(10, 100);
    seed_bounded_backlog(&harness.manager);
    assert_eq!(
        harness.manager.issue(&identity()).unwrap_err(),
        SessionError::AuditUnavailable
    );
    assert_eq!(
        scalar(
            &harness.manager,
            "SELECT count(*) FROM sessions WHERE state='active'"
        ),
        0
    );
    assert_eq!(
        scalar(
            &harness.manager,
            "SELECT count(*) FROM sessions WHERE state='pending' AND grant_receipt IS NULL"
        ),
        1
    );
    assert_eq!(
        scalar(
            &harness.manager,
            "SELECT total FROM auth_denials WHERE kind=6"
        ),
        1
    );
    assert_eq!(
        scalar(&harness.manager, "SELECT sum(total) FROM auth_denials"),
        1
    );
    assert_eq!(
        scalar(
            &harness.manager,
            "SELECT count(*) FROM audit_outbox WHERE receipt_json IS NOT NULL"
        ),
        256
    );
    let reopened = harness.reopen(10, 100);
    assert_eq!(reopened.flush_audit(256).unwrap(), 4);
    assert_eq!(
        scalar(&reopened, "SELECT total FROM auth_denials WHERE kind=6"),
        1
    );
    assert_eq!(
        scalar(&reopened, "SELECT delivered FROM auth_denials WHERE kind=6"),
        1
    );
    assert_eq!(
        scalar(
            &reopened,
            "SELECT count(*) FROM sessions WHERE state='active'"
        ),
        0
    );
    assert_eq!(
        scalar(
            &reopened,
            "SELECT count(*) FROM sessions WHERE state='pending' AND grant_receipt IS NOT NULL"
        ),
        1
    );
}
