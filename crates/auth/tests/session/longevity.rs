//! 不用 SQL 造会话：真实签发、吊销、维护超过旧累计上限，身份版本也越过 1025。

use super::*;
use std::collections::BTreeSet;
use studiolo_audit_ledger::{AuditFilter, AuditLedger};

#[test]
fn real_lifetime_cycles_exceed_1024_without_growing_auth_history_or_reviving_credentials() {
    let h = Harness::new(10, 100);
    let owner = identity();
    let mut first = None;
    let mut ids = BTreeSet::new();
    let mut reclaimed = 0;

    // 每轮仅一个存活记录，避免把这个长期行为测试变成全表二次方压力测试。
    for _ in 0..1026 {
        let issued = h.manager.issue(&owner).unwrap();
        assert!(ids.insert(issued.session.id.as_str().to_owned()));
        assert_eq!(
            h.manager
                .revoke_all(&owner, &issued.token, &issued.csrf)
                .unwrap()
                .revoked,
            1
        );
        let mut finished = false;
        for _ in 0..4 {
            let report = h.manager.maintain_retention().unwrap();
            assert!(report.reclaimed_sessions <= 16);
            reclaimed += report.reclaimed_sessions;
            if report.remaining == RetentionWork::None {
                finished = true;
                break;
            }
        }
        assert!(finished, "一轮有限终态必须能在有界维护中排空");
        assert_eq!(scalar(&h.manager, "SELECT count(*) FROM sessions"), 0);
        assert_eq!(scalar(&h.manager, "SELECT count(*) FROM audit_outbox"), 0);
        assert_eq!(
            scalar(&h.manager, "SELECT count(*) FROM auth_retention_plan"),
            0
        );
        assert_eq!(
            scalar(&h.manager, "SELECT count(*) FROM auth_retention_checkpoint"),
            1
        );
        assert_eq!(scalar(&h.manager, "SELECT count(*) FROM auth_denials"), 8);
        assert_eq!(
            scalar(&h.manager, "SELECT count(*) FROM credential_state"),
            1
        );
        assert_eq!(scalar(&h.manager, "SELECT count(*) FROM session_seeds"), 0);
        assert_eq!(scalar(&h.manager, "SELECT count(*) FROM auth_legacy"), 0);
        if first.is_none() {
            first = Some(issued);
        }
    }
    assert_eq!(reclaimed, 1026);
    assert_eq!(
        scalar(&h.manager, "SELECT epoch FROM auth_meta WHERE id=1"),
        1027
    );

    let reopened = h.reopen(10, 100);
    let latest = reopened.issue(&owner).unwrap();
    assert!(ids.insert(latest.session.id.as_str().to_owned()));
    let old = first.unwrap();
    assert_eq!(
        reopened.check(&owner, &old.token).unwrap_err(),
        SessionError::Rejected
    );
    assert_eq!(
        reopened
            .revoke(&owner, &latest.token, &old.csrf, &latest.session.id)
            .unwrap_err(),
        SessionError::Rejected
    );
    assert_eq!(
        reopened
            .revoke(&owner, &old.token, &old.csrf, &latest.session.id)
            .unwrap_err(),
        SessionError::Rejected
    );
    assert_eq!(
        reopened
            .revoke(&owner, &latest.token, &latest.csrf, &old.session.id)
            .unwrap()
            .revoked,
        0
    );
    assert_eq!(
        reopened.check(&owner, &latest.token).unwrap().state,
        SessionState::Active
    );
    assert_eq!(
        reopened.sessions(&owner, &latest.token).unwrap(),
        vec![latest.session]
    );
    assert_eq!(
        scalar(&reopened, "SELECT count(*) FROM auth_retention_checkpoint"),
        1
    );
    assert_eq!(
        scalar(&reopened, "SELECT count(*) FROM auth_retention_plan"),
        0
    );
    assert_eq!(
        scalar(&reopened, "SELECT count(*) FROM credential_state"),
        1
    );
    assert_eq!(scalar(&reopened, "SELECT count(*) FROM session_seeds"), 1);
    assert_eq!(scalar(&reopened, "SELECT count(*) FROM auth_legacy"), 0);

    // 历史移到权威审计；Auth 的行数有界不意味着审计可以丢弃旧证据。
    let grants = h
        .audit
        .query(&AuditFilter::default().with_action("session.issue"))
        .unwrap();
    assert_eq!(grants.len(), 1027 * 2);
    let revocations = h
        .audit
        .query(&AuditFilter::default().with_action("session.revoked"))
        .unwrap();
    assert_eq!(revocations.len(), 1026 * 2);
    assert!(
        !h.audit
            .query(&AuditFilter::default().with_action("session.compacted"))
            .unwrap()
            .is_empty()
    );
}
