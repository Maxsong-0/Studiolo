//! 双期限、持久回退、主人绑定与秘密展示的 Interface 回归。

use super::*;
use std::sync::atomic::Ordering;

#[test]
fn issue_reopen_and_metadata_never_expose_secrets() {
    let harness = Harness::new(10, 100);
    let issued = harness.manager.issue(&identity()).unwrap();
    let text = format!("{issued:?}");
    assert!(!text.contains(issued.token.expose_secret()));
    assert!(!text.contains(issued.csrf.expose_secret()));
    assert_ne!(issued.token.expose_secret(), issued.csrf.expose_secret());
    let reopened = harness.reopen(10, 100);
    assert_eq!(
        reopened.check(&identity(), &issued.token).unwrap(),
        issued.session
    );
    let metadata = format!(
        "{:?}",
        reopened.sessions(&identity(), &issued.token).unwrap()
    );
    assert!(!metadata.contains(&crate::session::secrets::hex(
        &crate::session::secrets::token_digest(&issued.token)
    )));
    let connection = harness.manager.database.connection.lock().unwrap();
    let stored: String = connection
        .query_row(
            "SELECT group_concat(event_json) FROM audit_outbox",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!stored.contains(issued.token.expose_secret()));
    assert!(!stored.contains(issued.csrf.expose_secret()));
}

#[test]
fn idle_boundary_is_durable_and_clock_rollback_cannot_resurrect() {
    let h = Harness::new(10, 100);
    let issued = h.manager.issue(&identity()).unwrap();
    h.clock.set(1_009_999);
    h.manager.check(&identity(), &issued.token).unwrap();
    h.clock.set(1_019_999);
    assert_eq!(
        h.manager.check(&identity(), &issued.token).unwrap_err(),
        SessionError::Rejected
    );
    assert_eq!(
        scalar(
            &h.manager,
            "SELECT count(*) FROM sessions WHERE state='expired'"
        ),
        1
    );
    h.clock.set(1_010_000);
    assert_eq!(
        h.manager.check(&identity(), &issued.token).unwrap_err(),
        SessionError::ClockRollback
    );
    assert!(matches!(
        SessionManager::open(
            &h.root,
            config(10, 100),
            h.audit.clone(),
            h.clock.clone(),
            h.random.clone()
        ),
        Err(SessionError::ClockRollback)
    ));
    h.clock.set(1_020_000);
    assert_eq!(
        h.reopen(10, 100)
            .check(&identity(), &issued.token)
            .unwrap_err(),
        SessionError::Rejected
    );
}

#[test]
fn absolute_expiry_never_slides_and_initial_access_caps_it() {
    let h = Harness::new(10, 20);
    let issued = h.manager.issue(&identity()).unwrap();
    h.clock.set(1_009_000);
    h.manager.check(&identity(), &issued.token).unwrap();
    h.clock.set(1_018_000);
    h.manager.check(&identity(), &issued.token).unwrap();
    h.clock.set(1_020_000);
    assert_eq!(
        h.manager.check(&identity(), &issued.token).unwrap_err(),
        SessionError::Rejected
    );
    let h = Harness::new(10, 100);
    let short = crate::access::test_identity_with("fictional-person", 1005);
    let issued = h.manager.issue(&short).unwrap();
    assert_eq!(issued.session.absolute_expires_at_ms, 1_005_000);
    h.clock.set(1_005_000);
    assert_eq!(
        h.manager.check(&identity(), &issued.token).unwrap_err(),
        SessionError::Rejected
    );
}

#[test]
fn owner_csrf_and_config_changes_fail_closed() {
    let h = Harness::new(10, 100);
    let first = h.manager.issue(&identity()).unwrap();
    let second = h.manager.issue(&identity()).unwrap();
    let other = crate::access::test_identity_with("someone-else", 4102444800);
    assert_eq!(
        h.manager.check(&other, &first.token).unwrap_err(),
        SessionError::Rejected
    );
    assert_eq!(
        h.manager
            .revoke(&identity(), &first.token, &second.csrf, &second.session.id)
            .unwrap_err(),
        SessionError::Rejected
    );
    h.manager.check(&identity(), &second.token).unwrap();
    assert!(matches!(
        SessionManager::open(
            &h.root,
            config(11, 100),
            h.audit.clone(),
            h.clock.clone(),
            h.random.clone()
        ),
        Err(SessionError::Configuration)
    ));
    assert!(SessionPolicy::new(0, 100).is_err());
    assert!(SessionPolicy::new(101, 100).is_err());
    assert!(SessionPolicy::new(1, 604801).is_err());
}

#[test]
fn random_failure_and_repeated_secret_values_never_grant() {
    let h = Harness::new(10, 100);
    h.random.fail.store(true, Ordering::SeqCst);
    assert_eq!(
        h.manager.issue(&identity()).unwrap_err(),
        SessionError::RandomUnavailable
    );
    assert_eq!(
        scalar(&h.manager, "SELECT total FROM auth_denials WHERE kind=6"),
        1
    );
    assert_eq!(scalar(&h.manager, "SELECT count(*) FROM sessions"), 0);
    h.random.fail.store(false, Ordering::SeqCst);
    let before = h.random.next.load(Ordering::SeqCst);
    h.manager.issue(&identity()).unwrap();
    h.random.next.store(before, Ordering::SeqCst);
    assert_eq!(
        h.manager.issue(&identity()).unwrap_err(),
        SessionError::RandomCollision
    );
    assert_eq!(
        scalar(
            &h.manager,
            "SELECT count(*) FROM sessions WHERE state='active'"
        ),
        1
    );
}
