//! 规范编码、标准派生向量与持久签发序号；所有秘密均为虚构夹具。

use super::*;
use std::sync::atomic::{AtomicU8, Ordering};

struct FixedSeeds(AtomicU8);
impl SessionRandom for FixedSeeds {
    fn fill(&self, output: &mut [u8]) -> Result<(), SessionError> {
        output.fill(self.0.fetch_add(1, Ordering::SeqCst));
        Ok(())
    }
}

#[test]
fn rfc5869_sha256_case_one_matches_the_locked_adapter() {
    let mut output = [0_u8; 42];
    let salt: Vec<u8> = (0..=12).collect();
    let info: Vec<u8> = (240..=249).collect();
    hkdf::Hkdf::<sha2_legacy::Sha256>::new(Some(&salt), &[11; 22])
        .expand(&info, &mut output)
        .unwrap();
    assert_eq!(
        crate::session::secrets::hex(&output),
        concat!(
            "3cb25f25faacd57a90434f64d0362f2a2d2d0a90cf1a5a4c5db02d56ecc4c5bf3400",
            "7208d5b887185865"
        )
    );
}

#[test]
fn v2_derivation_binds_store_sequence_and_purpose_without_exposing_seeds() {
    let generate = |store: &str, sequence| {
        crate::session::secrets::generate(&FixedSeeds(AtomicU8::new(17)), store, sequence).unwrap()
    };
    let first = generate("000102030405060708090a0b0c0d0e0f", 42);
    // 独立 Python 标准库 HMAC 计算的固定向量，不从受测函数计算期望值。
    assert_eq!(
        first.token.expose_secret(),
        concat!(
            "s2.000000000000002a.",
            "6edfe5ff4a0865e1479cfb12abdba96208efe4187374e80d894dde0f13830e78"
        )
    );
    assert_ne!(first.token.expose_secret(), "11".repeat(32));
    assert_ne!(
        &first.token.expose_secret()[20..],
        &first.csrf.expose_secret()[20..]
    );
    for other in [
        generate("000102030405060708090a0b0c0d0e0f", 43),
        generate("100102030405060708090a0b0c0d0e0f", 42),
    ] {
        assert_ne!(first.token.expose_secret(), other.token.expose_secret());
    }
}

#[test]
fn credential_and_identifier_encodings_reject_noncanonical_or_wrong_roles() {
    let suffix = "a".repeat(64);
    for sequence in [
        "0000000000000000",
        "8000000000000000",
        "000000000000000A",
        "1",
    ] {
        assert!(SessionId::parse(format!("v2-{sequence}")).is_err());
        assert!(SessionToken::from_hex(format!("s2.{sequence}.{suffix}")).is_err());
    }
    let valid = format!("s2.0000000000000001.{suffix}");
    assert!(SessionToken::from_hex(valid.clone()).is_ok());
    assert!(CsrfToken::from_hex(valid.clone()).is_err());
    for invalid in [
        format!(" {valid}"),
        format!("{valid}."),
        valid.to_uppercase(),
    ] {
        assert!(SessionToken::from_hex(invalid).is_err());
    }
    assert!(SessionToken::from_hex(suffix.clone()).is_ok());
    assert!(CsrfToken::from_hex(suffix).is_ok());
    assert!(SessionId::parse("0".repeat(32)).is_ok());
}

#[test]
fn failed_intent_burns_issuance_and_reopen_never_reuses_it() {
    let h = Harness::new(10, 100);
    h.audit.fail_intent.store(true, Ordering::SeqCst);
    assert_eq!(
        h.manager.issue(&identity()).unwrap_err(),
        SessionError::AuditUnavailable
    );
    assert_eq!(
        scalar(&h.manager, "SELECT next_issuance FROM credential_state"),
        2
    );
    h.audit.fail_intent.store(false, Ordering::SeqCst);
    let reopened = h.reopen(10, 100);
    let issued = reopened.issue(&identity()).unwrap();
    assert_eq!(issued.session.id.as_str(), "v2-0000000000000002");
    assert_eq!(
        scalar(&reopened, "SELECT next_issuance FROM credential_state"),
        3
    );
}

#[test]
fn issuance_exhaustion_fails_before_an_external_grant() {
    let h = Harness::new(10, 100);
    h.manager
        .database
        .connection
        .lock()
        .unwrap()
        .execute("UPDATE credential_state SET next_issuance=?1", [i64::MAX])
        .unwrap();
    assert_eq!(
        h.manager.issue(&identity()).unwrap_err(),
        SessionError::Capacity
    );
    assert_eq!(scalar(&h.manager, "SELECT count(*) FROM sessions"), 0);
    assert_eq!(
        scalar(&h.manager, "SELECT total FROM auth_denials WHERE kind=7"),
        1
    );
    assert_eq!(
        scalar(&h.manager, "SELECT next_issuance FROM credential_state"),
        i64::MAX
    );
    assert!(
        h.audit
            .query(&studiolo_audit_ledger::AuditFilter::default().with_action("session.issue"))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn issuance_exhaustion_commits_observed_expiry_and_clock_floor_before_rejecting() {
    let h = Harness::new(10, 100);
    let issued = h.manager.issue(&identity()).unwrap();
    h.manager
        .database
        .connection
        .lock()
        .unwrap()
        .execute("UPDATE credential_state SET next_issuance=?1", [i64::MAX])
        .unwrap();
    h.clock.set(1_020_000);
    assert_eq!(
        h.manager.issue(&identity()).unwrap_err(),
        SessionError::Capacity
    );
    assert_eq!(
        scalar(&h.manager, "SELECT clock_floor FROM auth_meta"),
        1_020_000
    );
    assert_eq!(
        scalar(
            &h.manager,
            "SELECT count(*) FROM sessions WHERE state='expired'"
        ),
        1
    );
    assert_eq!(
        scalar(&h.manager, "SELECT total FROM auth_denials WHERE kind=7"),
        1
    );
    h.clock.set(1_005_000);
    assert_eq!(
        h.manager.check(&identity(), &issued.token).unwrap_err(),
        SessionError::ClockRollback
    );
}

#[test]
fn missing_seed_or_issuance_rollback_cannot_authenticate_an_existing_session() {
    for mutation in [
        "DELETE FROM session_seeds",
        "UPDATE session_seeds SET issuance=issuance+1",
        "UPDATE credential_state SET next_issuance=1",
        "UPDATE session_seeds SET token_seed=zeroblob(31)",
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
            "{mutation}"
        );
    }
}
