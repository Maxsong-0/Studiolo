//! Auth 与 Audit 均用真实私有 SQLite；回收、审计故障和两库重开形成完整闭环。

use super::signed_identity::DurableHarness;
use super::*;
use std::{fs, os::unix::fs::PermissionsExt};
use studiolo_audit_ledger::{AuditFilter, AuditLedger};

#[test]
fn real_two_store_retention_survives_audit_outage_and_both_stores_reopen() {
    let h = DurableHarness::new(1_000_000);
    let owner = identity();
    let controller = h.manager.issue(&owner).unwrap();
    let first = h.manager.issue(&owner).unwrap();
    let second = h.manager.issue(&owner).unwrap();
    h.manager
        .revoke(
            &owner,
            &controller.token,
            &controller.csrf,
            &first.session.id,
        )
        .unwrap();
    let before = h.audit.query(&AuditFilter::default()).unwrap();
    assert_eq!(
        h.manager.maintain_retention().unwrap().reclaimed_sessions,
        1
    );
    let h = h.reopen_both();
    assert_eq!(
        h.manager.check(&owner, &first.token).unwrap_err(),
        SessionError::Rejected
    );
    assert_eq!(
        h.manager.check(&owner, &controller.token).unwrap().state,
        SessionState::Active
    );

    // 仅收紧测试拥有的 Audit 文件权限；合法吊销仍先提交到 Auth。
    fs::set_permissions(h.audit_file(), fs::Permissions::from_mode(0o400)).unwrap();
    assert_eq!(
        h.manager
            .revoke(
                &owner,
                &controller.token,
                &controller.csrf,
                &second.session.id
            )
            .unwrap()
            .revoked,
        1
    );
    assert_eq!(
        h.manager.maintain_retention().unwrap_err(),
        SessionError::AuditUnavailable
    );
    assert_eq!(scalar(&h.manager, "SELECT count(*) FROM sessions"), 2);
    let h = h.reopen_auth();
    assert_eq!(
        h.manager.check(&owner, &second.token).unwrap_err(),
        SessionError::Rejected
    );
    assert_eq!(
        scalar(
            &h.manager,
            "SELECT count(*) FROM sessions WHERE state='revoked'"
        ),
        1
    );
    fs::set_permissions(h.audit_file(), fs::Permissions::from_mode(0o600)).unwrap();
    let h = h.reopen_both();
    assert_eq!(
        h.manager.maintain_retention().unwrap().reclaimed_sessions,
        1
    );
    assert_eq!(
        h.manager.sessions(&owner, &controller.token).unwrap(),
        vec![controller.session]
    );
    assert_eq!(
        scalar(&h.manager, "SELECT count(*) FROM auth_retention_checkpoint"),
        1
    );
    assert_eq!(
        scalar(&h.manager, "SELECT count(*) FROM auth_retention_plan"),
        0
    );
    let after = h.audit.query(&AuditFilter::default()).unwrap();
    assert_eq!(&after[..before.len()], before.as_slice());
    assert_eq!(
        h.audit
            .query(&AuditFilter::default().with_action("session.compacted"))
            .unwrap()
            .len(),
        2
    );
    for record in &after {
        record.verify_integrity().unwrap();
        assert_eq!(h.audit.append(record.event().clone()).unwrap(), *record);
    }
    let h = h.reopen_both();
    assert_eq!(h.audit.query(&AuditFilter::default()).unwrap(), after);
    assert_eq!(
        h.manager.maintain_retention().unwrap().reclaimed_sessions,
        0
    );
    assert_eq!(
        h.manager.check(&owner, &first.token).unwrap_err(),
        SessionError::Rejected
    );
    assert_eq!(
        h.manager.check(&owner, &second.token).unwrap_err(),
        SessionError::Rejected
    );
    assert_eq!(
        h.manager.check(&owner, &controller.token).unwrap().state,
        SessionState::Active
    );
}
