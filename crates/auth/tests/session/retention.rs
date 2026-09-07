//! 通过维护 Interface 检验固定批量及未交付证据；不把 SQL 夹具当成长期签发验证。

use super::*;
use std::sync::atomic::Ordering;

#[test]
fn maintenance_reclaims_at_most_sixteen_and_keeps_active_session_usable() {
    let h = Harness::new(10, 100);
    let owner = identity();
    let controller = h.manager.issue(&owner).unwrap();
    let mut retired = Vec::new();
    for _ in 0..17 {
        let issued = h.manager.issue(&owner).unwrap();
        assert_eq!(
            h.manager
                .revoke(
                    &owner,
                    &controller.token,
                    &controller.csrf,
                    &issued.session.id
                )
                .unwrap()
                .revoked,
            1
        );
        retired.push(issued);
    }
    let first = h.manager.maintain_retention().unwrap();
    assert_eq!(first.reclaimed_sessions, 16);
    assert_eq!(first.remaining, RetentionWork::More);
    assert_eq!(scalar(&h.manager, "SELECT count(*) FROM sessions"), 2);
    assert_eq!(
        h.manager.check(&owner, &controller.token).unwrap().state,
        SessionState::Active
    );
    let second = h.manager.maintain_retention().unwrap();
    assert_eq!(second.reclaimed_sessions, 1);
    assert_eq!(second.remaining, RetentionWork::None);
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
    for issued in retired {
        assert_eq!(
            h.manager.check(&owner, &issued.token).unwrap_err(),
            SessionError::Rejected
        );
    }
}

#[test]
fn unacknowledged_terminal_survives_outage_and_does_not_block_reads_or_revocation() {
    let h = Harness::new(10, 100);
    let owner = identity();
    let controller = h.manager.issue(&owner).unwrap();
    let first = h.manager.issue(&owner).unwrap();
    let second = h.manager.issue(&owner).unwrap();
    h.audit.fail.store(true, Ordering::SeqCst);
    assert_eq!(
        h.manager
            .revoke(
                &owner,
                &controller.token,
                &controller.csrf,
                &first.session.id
            )
            .unwrap()
            .revoked,
        1
    );
    assert_eq!(
        h.manager.maintain_retention().unwrap_err(),
        SessionError::AuditUnavailable
    );
    assert_eq!(scalar(&h.manager, "SELECT count(*) FROM sessions"), 3);
    assert_eq!(
        scalar(
            &h.manager,
            "SELECT count(*) FROM audit_outbox WHERE receipt_json IS NULL"
        ),
        2
    );
    assert_eq!(
        h.manager.check(&owner, &controller.token).unwrap().state,
        SessionState::Active
    );
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
        scalar(
            &h.manager,
            "SELECT count(*) FROM sessions WHERE state='revoked'"
        ),
        2
    );
    let reopened = h.reopen(10, 100);
    assert_eq!(scalar(&reopened, "SELECT count(*) FROM sessions"), 3);
    assert_eq!(
        reopened.check(&owner, &first.token).unwrap_err(),
        SessionError::Rejected
    );
    assert_eq!(
        reopened.check(&owner, &second.token).unwrap_err(),
        SessionError::Rejected
    );
    h.audit.fail.store(false, Ordering::SeqCst);
    assert_eq!(reopened.maintain_retention().unwrap().reclaimed_sessions, 2);
    assert_eq!(
        reopened.sessions(&owner, &controller.token).unwrap(),
        vec![controller.session]
    );
}

#[test]
fn repeated_old_random_seeds_after_collection_do_not_reproduce_old_credentials() {
    let h = Harness::new(10, 100);
    let owner = identity();
    let first_seed = h.random.next.load(Ordering::SeqCst);
    let old = h.manager.issue(&owner).unwrap();
    h.manager.revoke_all(&owner, &old.token, &old.csrf).unwrap();
    assert_eq!(
        h.manager.maintain_retention().unwrap().reclaimed_sessions,
        1
    );
    assert_eq!(scalar(&h.manager, "SELECT count(*) FROM session_seeds"), 0);
    h.random.next.store(first_seed, Ordering::SeqCst);
    let fresh = h.manager.issue(&owner).unwrap();
    assert_ne!(fresh.session.id, old.session.id);
    // 不把秘密放进断言失败输出；序号参与派生后，重复种子也不能重现旧传输值。
    assert!(fresh.token.expose_secret() != old.token.expose_secret());
    assert!(fresh.csrf.expose_secret() != old.csrf.expose_secret());
    assert_eq!(
        h.manager.check(&owner, &old.token).unwrap_err(),
        SessionError::Rejected
    );
    assert_eq!(
        h.manager
            .revoke(&owner, &fresh.token, &old.csrf, &fresh.session.id)
            .unwrap_err(),
        SessionError::Rejected
    );
    assert_eq!(
        h.manager.check(&owner, &fresh.token).unwrap().state,
        SessionState::Active
    );
}
