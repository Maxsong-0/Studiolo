//! 单字段损坏、聚合补记计数关联以及真实 SQLite 写失败。

use super::*;

#[test]
fn damaged_schema_config_session_or_receipt_fails_on_the_first_check() {
    for mutation in [
        "PRAGMA user_version=3",
        "PRAGMA application_id=0",
        "CREATE TABLE extra(value TEXT)",
        "UPDATE auth_meta SET config='{}'",
        "UPDATE auth_meta SET clock_floor=0",
        "UPDATE sessions SET grant_receipt='{}'",
        "UPDATE sessions SET grant_id='other'",
        "UPDATE sessions SET epoch=99",
        "UPDATE sessions SET absolute_expiry=absolute_expiry-1",
        "UPDATE audit_outbox SET receipt_json='{}'",
        "UPDATE audit_outbox SET event_json='{}'",
        "UPDATE audit_outbox SET event_json=CAST(zeroblob(1000000) AS TEXT)",
    ] {
        let h = Harness::new(10, 100);
        let issued = h.manager.issue(&identity()).unwrap();
        h.manager
            .database
            .connection
            .lock()
            .unwrap()
            .execute_batch(mutation)
            .unwrap();
        assert!(
            h.manager.check(&identity(), &issued.token).is_err(),
            "accepted {mutation}"
        );
        assert!(
            SessionManager::open(
                &h.root,
                config(10, 100),
                h.audit.clone(),
                h.clock.clone(),
                h.random.clone()
            )
            .is_err()
        );
    }
}

#[test]
fn altered_denial_bucket_or_snapshot_cannot_acknowledge_unlogged_counts() {
    for mutation in [
        "UPDATE audit_outbox SET through_count=10 WHERE bucket=2",
        "UPDATE audit_outbox SET through_count=NULL WHERE bucket=2",
        "UPDATE audit_outbox SET bucket=3 WHERE bucket=2",
    ] {
        let h = Harness::new(10, 100);
        let bad = SessionToken::from_hex("f".repeat(64)).unwrap();
        for _ in 0..10 {
            assert!(h.manager.check(&identity(), &bad).is_err());
        }
        h.manager
            .database
            .connection
            .lock()
            .unwrap()
            .execute_batch(mutation)
            .unwrap();
        assert!(h.manager.flush_audit(256).is_err());
        assert_eq!(
            scalar(
                &h.manager,
                "SELECT delivered FROM auth_denials WHERE kind=2"
            ),
            0
        );
    }
}

#[test]
fn revoked_state_cannot_be_flipped_back_to_active_even_with_valid_grant_receipt() {
    let h = Harness::new(10, 100);
    let owner = h.manager.issue(&identity()).unwrap();
    let victim = h.manager.issue(&identity()).unwrap();
    h.manager
        .revoke(&identity(), &owner.token, &owner.csrf, &victim.session.id)
        .unwrap();
    h.manager
        .database
        .connection
        .lock()
        .unwrap()
        .execute(
            "UPDATE sessions SET state='active' WHERE id=?1",
            [victim.session.id.as_str()],
        )
        .unwrap();
    assert_eq!(
        h.manager.check(&identity(), &victim.token).unwrap_err(),
        SessionError::CorruptStore
    );
}
