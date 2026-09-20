//! 仅虚构离线恢复副本：真实 SQLite、原审计回执与持久幂等。
use super::*;
use std::sync::{Arc, atomic::Ordering};
use studiolo_audit_ledger::{AuditFilter, AuditLedger, AuditPhase, MemoryAuditLedger};

#[path = "restore_failures.rs"]
mod failures;
#[path = "restore_limits.rs"]
mod limits;
#[path = "restore_payload.rs"]
mod payload;

fn rows(manager: &SessionManager, sql: &str) -> Vec<String> {
    let db = manager.database.connection.lock().unwrap();
    let mut query = db.prepare(sql).unwrap();
    let columns = query.column_count();
    query
        .query_map([], |row| {
            let values = (0..columns)
                .map(|i| row.get::<_, rusqlite::types::Value>(i))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(format!("{values:?}"))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}

fn preserved(manager: &SessionManager) -> Vec<Vec<String>> {
    [
        "SELECT * FROM totp_factor",
        "SELECT next_operation,change_json,consumption_json,high_step FROM recovery_control",
        "SELECT * FROM totp_rates",
        "SELECT * FROM recovery_rates",
        "SELECT clock_floor,store_id FROM auth_meta",
        "SELECT * FROM credential_state",
        "SELECT * FROM auth_denials",
    ]
    .into_iter()
    .map(|sql| rows(manager, sql))
    .collect()
}

fn whole(manager: &SessionManager) -> Vec<Vec<String>> {
    let names = rows(
        manager,
        "SELECT name FROM sqlite_schema WHERE type='table' ORDER BY name",
    );
    // 表名来自受控固定 schema；断言助手不对任意外部 SQL 开放。
    let db = manager.database.connection.lock().unwrap();
    let mut query = db
        .prepare("SELECT name FROM sqlite_schema WHERE type='table' ORDER BY name")
        .unwrap();
    let tables = query
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    drop(query);
    drop(db);
    assert_eq!(names.len(), tables.len());
    tables
        .iter()
        .map(|name| rows(manager, &format!("SELECT * FROM {name} ORDER BY rowid")))
        .collect()
}

#[test]
fn zero_sessions_keeps_factor_and_clock_and_reuses_id_after_retention() {
    let h = RecoveryHarness::new(3600, 7200);
    let before = preserved(&h.h.manager);
    h.h.clock.set(CHANGE);
    let first =
        h.h.manager
            .invalidate_restored_credentials("zero-1")
            .unwrap();
    assert_eq!(
        first,
        RestoreInvalidation {
            epoch: 2,
            revoked_sessions: 0,
            cleared_l3: 0,
            invalidated_recovery_codes: 0,
            audit_confirmed: true
        }
    );
    assert_eq!(preserved(&h.h.manager), before);
    h.h.manager.maintain_retention().unwrap();
    let fresh = h.h.manager.issue(&identity()).unwrap();
    let second =
        h.h.manager
            .invalidate_restored_credentials("zero-1")
            .unwrap();
    assert_eq!(first, second);
    h.h.manager.check(&identity(), &fresh.token).unwrap();
    let reopened = h.h.reopen(3600, 7200);
    assert_eq!(
        reopened.invalidate_restored_credentials("zero-1").unwrap(),
        first
    );
    assert_eq!(
        scalar(
            &reopened,
            "SELECT count(*) FROM audit_outbox WHERE event_id LIKE '%:restore:%'"
        ),
        2
    );
}

#[test]
fn restores_real_replaced_factor_preserves_proofs_and_retires_all_old_authority() {
    let h = RecoveryHarness::new(3600, 7200);
    let owner = identity();
    let (old, codes) = h.ready();
    let prepared = h.prepared();
    h.h.clock.set(CHANGE);
    let replacement = h
        .recovery
        .replace_factor(
            &request(&owner, &old, 1),
            copy_code(&codes, 0),
            &prepared,
            otp(&NEW, CHANGE),
        )
        .unwrap();
    h.h.clock.set(CHANGE + 30_000);
    let active = h.h.manager.issue(&owner).unwrap();
    let req = request(&owner, &active, 1);
    replacement
        .step_up(&req, otp(&NEW, CHANGE + 30_000))
        .unwrap();
    let old_codes = h.recovery.rotate_codes(&req).unwrap();
    let another = h.h.manager.issue(&owner).unwrap();
    // 一条真实 issuance 在审计故障后保持 pending，也必须进入撤权计数。
    h.h.audit.fail_outcome.store(true, Ordering::SeqCst);
    assert!(h.h.manager.issue(&owner).is_err());
    h.h.audit.fail_outcome.store(false, Ordering::SeqCst);
    assert_eq!(
        scalar(
            &h.h.manager,
            "SELECT count(*) FROM sessions WHERE state='pending'"
        ),
        1
    );
    let before = preserved(&h.h.manager);
    let history = scalar(&h.h.manager, "SELECT count(*) FROM sessions");
    let result =
        h.h.manager
            .invalidate_restored_credentials("copy-20260920")
            .unwrap();
    assert_eq!(
        result,
        RestoreInvalidation {
            epoch: 3,
            revoked_sessions: 3,
            cleared_l3: 1,
            invalidated_recovery_codes: 10,
            audit_confirmed: true
        }
    );
    assert_eq!(preserved(&h.h.manager), before);
    assert_eq!(
        scalar(&h.h.manager, "SELECT count(*) FROM sessions"),
        history
    );
    assert_eq!(
        scalar(
            &h.h.manager,
            "SELECT count(*) FROM sessions WHERE state IN ('active','pending')"
        ),
        0
    );
    assert_eq!(scalar(&h.h.manager, "SELECT count(*) FROM totp_grants"), 0);
    assert_eq!(
        scalar(&h.h.manager, "SELECT count(*) FROM recovery_codes"),
        0
    );
    assert_eq!(
        scalar(
            &h.h.manager,
            "SELECT current_json IS NULL AND used_mask=0 FROM recovery_control"
        ),
        1
    );
    for issued in [&old, &active, &another] {
        assert_eq!(
            h.h.manager.check(&owner, &issued.token).unwrap_err(),
            SessionError::Rejected
        );
        assert!(
            replacement
                .require_l3(&request(&owner, issued, 1), SensitiveAction::RestoreVault)
                .is_err()
        );
    }
    let (manager, recovery) = h.reopen();
    let (factor, _) = Totp::attach(manager.clone(), seed(NEW)).unwrap();
    let fresh = manager.issue(&owner).unwrap();
    let req = request(&owner, &fresh, 1);
    assert_eq!(
        factor
            .require_l3(&req, SensitiveAction::ManageRecoveryCodes)
            .unwrap_err(),
        SessionError::ReauthenticationRequired
    );
    h.h.clock.set(CHANGE + 60_000);
    factor.step_up(&req, otp(&NEW, CHANGE + 60_000)).unwrap();
    assert_eq!(recovery.rotate_codes(&req).unwrap().codes().len(), 10);
    let third = [99; 20];
    let candidate = PreparedTotpFactor::prepare(
        manager.clone(),
        TotpSeed::new("test-only-third".into(), third).unwrap(),
    )
    .unwrap();
    assert_eq!(
        recovery
            .replace_factor(
                &req,
                copy_code(&old_codes, 0),
                &candidate,
                otp(&third, CHANGE + 60_000)
            )
            .err()
            .unwrap(),
        SessionError::Rejected
    );
    for _ in 0..4 {
        manager.maintain_retention().unwrap();
    }
    assert_eq!(
        manager
            .invalidate_restored_credentials("copy-20260920")
            .unwrap(),
        result
    );
    manager.check(&owner, &fresh.token).unwrap();
    assert_eq!(
        scalar(
            &manager,
            "SELECT count(*) FROM audit_outbox WHERE event_id LIKE '%:restore:%'"
        ),
        2
    );
}
