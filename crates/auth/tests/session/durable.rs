//! 双私有 SQLite 的故障闭环；仅改变本次审计夹具的文件权限。

use super::signed_identity::{DurableHarness, stored_text};
use super::*;
use std::{fs, os::unix::fs::PermissionsExt};
use studiolo_audit_ledger::{
    AuditError, AuditEvent, AuditFilter, AuditLedger, AuditPhase, AuditRecord,
};

#[test]
fn revocation_replays_exactly_after_real_audit_outage_and_both_stores_reopen() {
    let harness = DurableHarness::new(1_000_000);
    let identity = identity();
    let controller = harness.manager.issue(&identity).unwrap();
    let target = harness.manager.issue(&identity).unwrap();
    let before = harness.audit.query(&AuditFilter::default()).unwrap();
    assert_eq!(before.len(), 4);

    // 文件确实由真实审计 Adapter 拒绝；不替换为内存故障模拟器。
    fs::set_permissions(harness.audit_file(), fs::Permissions::from_mode(0o400)).unwrap();
    assert_eq!(
        harness.audit.query(&AuditFilter::default()).unwrap_err(),
        AuditError::UnsafeStorage
    );
    assert_eq!(
        harness
            .manager
            .sessions(&identity, &controller.token)
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        harness
            .manager
            .revoke(
                &identity,
                &controller.token,
                &controller.csrf,
                &target.session.id
            )
            .unwrap()
            .revoked,
        1
    );
    let originals = stored_text(
        &harness.manager,
        "SELECT event_json FROM audit_outbox WHERE kind='revoke' ORDER BY sequence",
    );
    assert_eq!(originals.len(), 2);
    assert_eq!(
        scalar(
            &harness.manager,
            "SELECT count(*) FROM audit_outbox WHERE kind='revoke' AND receipt_json IS NULL"
        ),
        2
    );

    // Auth 在审计仍不可用时可重开，已提交的吊销不会回滚。
    let reopened = harness.reopen_auth();
    assert_eq!(
        reopened
            .manager
            .check(&identity, &target.token)
            .unwrap_err(),
        SessionError::Rejected
    );
    assert_eq!(
        reopened
            .manager
            .check(&identity, &controller.token)
            .unwrap()
            .state,
        SessionState::Active
    );
    assert_eq!(
        stored_text(
            &reopened.manager,
            "SELECT event_json FROM audit_outbox WHERE kind='revoke' ORDER BY sequence"
        ),
        originals
    );
    fs::set_permissions(reopened.audit_file(), fs::Permissions::from_mode(0o600)).unwrap();
    let reopened = reopened.reopen_both();
    assert_eq!(
        reopened.audit.query(&AuditFilter::default()).unwrap(),
        before
    );

    // 先只送一条，再重开两库，确认不会丢掉另一阶段或改写原事件。
    assert_eq!(reopened.manager.flush_audit(1).unwrap(), 1);
    let filter = AuditFilter::default().with_action("session.revoked");
    let partial = reopened.audit.query(&filter).unwrap();
    assert_eq!(partial.len(), 1);
    assert_eq!(partial[0].event().phase(), AuditPhase::Intent);
    let reopened = reopened.reopen_both();
    assert_eq!(reopened.manager.flush_audit(256).unwrap(), 2);
    let delivered = reopened.audit.query(&filter).unwrap();
    assert_eq!(delivered.len(), 2);
    assert_eq!(delivered[0], partial[0]);
    assert_eq!(delivered[1].event().phase(), AuditPhase::Outcome);
    for (record, original) in delivered.iter().zip(&originals) {
        let event = serde_json::from_str::<AuditEvent>(original).unwrap();
        assert_eq!(record.event(), &event);
        record.verify_integrity().unwrap();
        assert_eq!(reopened.audit.append(event).unwrap(), *record);
    }
    let receipts = stored_text(
        &reopened.manager,
        "SELECT receipt_json FROM audit_outbox WHERE kind='revoke' ORDER BY sequence",
    );
    let receipts: Vec<AuditRecord> = receipts
        .iter()
        .map(|value| serde_json::from_str(value).unwrap())
        .collect();
    assert_eq!(receipts, delivered);
    assert_eq!(reopened.manager.flush_audit(256).unwrap(), 0);
    let after = reopened.audit.query(&AuditFilter::default()).unwrap();
    let reopened = reopened.reopen_both();
    assert_eq!(
        reopened.audit.query(&AuditFilter::default()).unwrap(),
        after
    );
    let sessions = reopened
        .manager
        .sessions(&identity, &controller.token)
        .unwrap();
    assert_eq!(
        sessions
            .iter()
            .find(|item| item.id == target.session.id)
            .unwrap()
            .state,
        SessionState::Revoked
    );
    assert_eq!(reopened.manager.flush_audit(256).unwrap(), 0);
}
