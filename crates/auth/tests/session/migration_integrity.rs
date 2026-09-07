//! 迁移拒绝和旧摘要保留的负向契约；所有 SQL 篡改仅用于虚构临时数据库。

use super::{super::*, LegacyFixture};
use sha2::{Digest, Sha256};

#[test]
fn corrupt_or_unknown_v1_is_not_repaired_or_partially_migrated() {
    for mutation in [
        "PRAGMA user_version=99",
        "PRAGMA application_id=0",
        "CREATE TABLE unexpected(value TEXT)",
        "UPDATE auth_meta SET clock_floor=0",
        "UPDATE sessions SET grant_receipt='{}' WHERE state='active'",
        "UPDATE audit_outbox SET event_json='{}'",
    ] {
        let old = LegacyFixture::new();
        old.h
            .manager
            .database
            .connection
            .lock()
            .unwrap()
            .execute_batch(mutation)
            .unwrap();
        let before = old.snapshot();
        let result = SessionManager::migrate_v1(
            &old.h.root,
            config(10, 100),
            old.h.audit.clone(),
            old.h.clock.clone(),
            old.h.random.clone(),
        );
        assert!(result.is_err(), "未拒绝损坏夹具：{mutation}");
        assert_eq!(old.snapshot(), before);
        assert_eq!(
            scalar(
                &old.h.manager,
                "SELECT count(*) FROM sqlite_schema WHERE name='credential_state'"
            ),
            0
        );
    }
}

#[test]
fn mismatched_configuration_cannot_migrate_existing_owner_or_policy() {
    let old = LegacyFixture::new();
    let before = old.snapshot();
    let result = SessionManager::migrate_v1(
        &old.h.root,
        config(11, 100),
        old.h.audit.clone(),
        old.h.clock.clone(),
        old.h.random.clone(),
    );
    assert_eq!(result.err().unwrap(), SessionError::Configuration);
    assert_eq!(old.snapshot(), before);
    assert_eq!(scalar(&old.h.manager, "PRAGMA user_version"), 1);
}

#[test]
fn frozen_legacy_set_and_its_live_associations_are_exact() {
    for mutation in [
        "UPDATE credential_state SET legacy_count=1",
        "UPDATE credential_state SET legacy_hash=zeroblob(32)",
        "DELETE FROM auth_legacy WHERE id=(SELECT min(id) FROM auth_legacy)",
        "UPDATE auth_legacy SET token_digest=zeroblob(32)",
        "UPDATE sessions SET token_digest=zeroblob(32) WHERE state='active'",
    ] {
        let old = LegacyFixture::new();
        let manager = old.migrate();
        manager
            .database
            .connection
            .lock()
            .unwrap()
            .execute_batch(mutation)
            .unwrap();
        assert_eq!(
            manager.check(&identity(), &old.token).unwrap_err(),
            SessionError::CorruptStore
        );
        assert!(
            SessionManager::open(
                &old.h.root,
                config(10, 100),
                old.h.audit.clone(),
                old.h.clock.clone(),
                old.h.random.clone(),
            )
            .is_err()
        );
    }
}

#[test]
fn retained_v2_seeds_require_matching_ids_and_a_strictly_greater_counter() {
    for mutation in [
        "UPDATE credential_state SET next_issuance=1",
        "UPDATE session_seeds SET issuance=NULL",
        "UPDATE session_seeds SET id='v2-0000000000000002'",
        "UPDATE session_seeds SET token_seed=zeroblob(31)",
        "DELETE FROM session_seeds",
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
        assert_eq!(
            h.manager.check(&identity(), &issued.token).unwrap_err(),
            SessionError::CorruptStore
        );
    }
}

#[test]
fn legacy_seed_cross_domain_reuse_stays_rejected_after_original_sessions_are_reclaimed() {
    let old = LegacyFixture::new();
    let manager = old.migrate();
    manager
        .revoke_all(&identity(), &old.token, &old.csrf)
        .unwrap();
    manager.flush_audit(256).unwrap();
    assert_eq!(manager.maintain_retention().unwrap().reclaimed_sessions, 2);
    assert_eq!(scalar(&manager, "SELECT count(*) FROM sessions"), 0);
    assert_eq!(scalar(&manager, "SELECT count(*) FROM auth_legacy"), 2);
    let before = old
        .h
        .audit
        .query(&studiolo_audit_ledger::AuditFilter::default().with_action("session.issue"))
        .unwrap();
    for (token, csrf) in [(0x11, 0x77), (0x22, 0x77), (0x77, 0x11), (0x77, 0x22)] {
        let attempt = SessionManager::open(
            &old.h.root,
            config(10, 100),
            old.h.audit.clone(),
            old.h.clock.clone(),
            std::sync::Arc::new(LegacySeeds {
                token,
                csrf,
                calls: std::sync::atomic::AtomicUsize::new(0),
            }),
        )
        .unwrap();
        assert_eq!(
            attempt.issue(&identity()).unwrap_err(),
            SessionError::RandomCollision
        );
        assert_eq!(scalar(&attempt, "SELECT count(*) FROM sessions"), 0);
    }
    assert_eq!(
        old.h
            .audit
            .query(&studiolo_audit_ledger::AuditFilter::default().with_action("session.issue"))
            .unwrap(),
        before
    );
    assert!(
        !manager
            .database
            .transaction(|db| credential_store::collided(
                db,
                &seed_digest(true, 0x77),
                &seed_digest(false, 0x77),
                &seed_digest(true, 0x88),
                &seed_digest(false, 0x88),
            ))
            .unwrap()
    );
    assert_eq!(
        manager.check(&identity(), &old.token).unwrap_err(),
        SessionError::Rejected
    );
}

struct LegacySeeds {
    token: u8,
    csrf: u8,
    calls: std::sync::atomic::AtomicUsize,
}
impl SessionRandom for LegacySeeds {
    fn fill(&self, output: &mut [u8]) -> Result<(), SessionError> {
        let first = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0;
        output.fill(if first { self.token } else { self.csrf });
        Ok(())
    }
}

fn seed_digest(token_domain: bool, byte: u8) -> [u8; 32] {
    let domain: &[u8] = if token_domain {
        b"studiolo-session-v1\0"
    } else {
        b"studiolo-csrf-v1\0"
    };
    let mut hash = Sha256::new();
    hash.update(domain);
    hash.update(format!("{byte:02x}").repeat(32).as_bytes());
    hash.finalize().into()
}
