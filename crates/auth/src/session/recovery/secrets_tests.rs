//! 虚构随机输入、规范编码和域隔离；不读真实恢复材料。

use super::*;
use std::sync::atomic::{AtomicU8, Ordering};

const STORE: &str = "000102030405060708090a0b0c0d0e0f";

#[derive(Default)]
struct CountingRandom(AtomicU8);

impl SessionRandom for CountingRandom {
    fn fill(&self, output: &mut [u8]) -> Result<(), SessionError> {
        output.fill(self.0.fetch_add(1, Ordering::SeqCst) + 1);
        Ok(())
    }
}

#[test]
fn parser_accepts_only_fixed_ascii_positive_generation_and_ten_slots() {
    let valid = "r1.0000000000000001.09.0123456789abcdef0123456789abcdef";
    let code = RecoveryCode::parse(valid.to_owned()).unwrap();
    assert_eq!(code.expose_secret(), valid);
    assert_eq!(code.generation(), 1);
    assert_eq!(code.slot(), 9);
    for invalid in [
        valid.replace("r1.", "R1."),
        valid.replace(".0000000000000001.", ".0000000000000000."),
        valid.replace(".0000000000000001.", ".8000000000000000."),
        valid.replace(".09.", ".0a."),
        valid.replace(".09.", ".9."),
        valid.replace("abcdef", "ABCDEF"),
        valid.replacen("012345", "你好", 1),
        format!("{valid}\n"),
        valid[..54].to_owned(),
    ] {
        assert!(matches!(
            RecoveryCode::parse(invalid),
            Err(SessionError::Rejected)
        ));
    }
    let maximum = valid.replace("0000000000000001", "7fffffffffffffff");
    assert_eq!(RecoveryCode::parse(maximum).unwrap().generation(), i64::MAX);
}

#[test]
fn generation_draws_ten_separate_inputs_and_matches_independent_hmac_expansion() {
    use hmac::{Hmac, Mac};
    let random = CountingRandom::default();
    let generated = generate(&random, STORE, 1).unwrap();
    assert_eq!(random.0.load(Ordering::SeqCst), 10);
    assert_eq!(generated.codes.codes().len(), 10);
    assert_eq!(generated.digests.len(), 10);
    assert_eq!(generated.seed_digests.len(), 10);
    // 独立写出 HKDF 的 Extract 与第一轮 Expand，锁定盐与固定长 info 编码。
    let mut extract =
        Hmac::<sha2_legacy::Sha256>::new_from_slice(b"studiolo-recovery-v1\0").unwrap();
    extract.update(&[1; 32]);
    let prk = extract.finalize().into_bytes();
    let mut expand = Hmac::<sha2_legacy::Sha256>::new_from_slice(&prk).unwrap();
    expand.update(&(0_u8..16).collect::<Vec<_>>());
    expand.update(&1_i64.to_be_bytes());
    expand.update(&[0, 1]);
    let block = expand.finalize().into_bytes();
    let expected = format!(
        "r1.0000000000000001.00.{}",
        session_secrets::hex(&block[..16])
    );
    assert_eq!(generated.codes.codes()[0].expose_secret(), expected);
    for (slot, code) in generated.codes.codes().iter().enumerate() {
        assert_eq!(code.expose_secret().len(), 55);
        assert_eq!(usize::from(code.slot()), slot);
        assert_eq!(code.generation(), 1);
        assert_eq!(code.digest(STORE), generated.digests[slot]);
        assert_eq!(generated.digests[slot].len(), 64);
    }
}

#[test]
fn batch_store_and_slot_domains_differ_even_for_repeated_inputs_across_batches() {
    let first = generate(&CountingRandom::default(), STORE, 1).unwrap();
    let next = generate(&CountingRandom::default(), STORE, 2).unwrap();
    let other = generate(
        &CountingRandom::default(),
        "11111111111111111111111111111111",
        1,
    )
    .unwrap();
    for slot in 0..10 {
        assert_ne!(
            first.codes.codes()[slot].expose_secret(),
            next.codes.codes()[slot].expose_secret()
        );
        assert_ne!(first.digests[slot], other.digests[slot]);
        assert_eq!(first.seed_digests[slot], next.seed_digests[slot]);
    }
    let seed = [7; 32];
    assert_ne!(
        derive(&seed, STORE, 1, 0).unwrap(),
        derive(&seed, STORE, 1, 1).unwrap()
    );
    let code = &first.codes.codes()[0];
    assert_ne!(
        code.digest(STORE),
        code.digest("11111111111111111111111111111111")
    );
}

#[test]
fn original_seed_fingerprints_keep_existing_session_and_csrf_domains() {
    use crate::session::{CsrfToken, SessionToken};
    let generated = generate(&CountingRandom::default(), STORE, 1).unwrap();
    let raw_hex = session_secrets::hex(&[1; 32]);
    let session = SessionToken::from_hex(raw_hex.clone()).unwrap();
    let csrf = CsrfToken::from_hex(raw_hex).unwrap();
    assert_eq!(
        generated.seed_digests[0].0,
        session_secrets::token_digest(&session)
    );
    assert_eq!(
        generated.seed_digests[0].1,
        session_secrets::csrf_digest(&csrf)
    );
}

#[test]
fn invalid_context_random_failure_or_repeated_seed_never_returns_partial_codes() {
    struct FaultRandom(bool);
    impl SessionRandom for FaultRandom {
        fn fill(&self, output: &mut [u8]) -> Result<(), SessionError> {
            output.fill(3);
            if self.0 {
                Err(SessionError::RandomUnavailable)
            } else {
                Ok(())
            }
        }
    }
    let random = CountingRandom::default();
    for (store, generation) in [(STORE, 0), (STORE, -1), ("BAD", 1)] {
        assert!(matches!(
            generate(&random, store, generation),
            Err(SessionError::CorruptStore)
        ));
    }
    assert_eq!(random.0.load(Ordering::SeqCst), 0);
    assert!(matches!(
        generate(&FaultRandom(true), STORE, 1),
        Err(SessionError::RandomUnavailable)
    ));
    assert!(matches!(
        generate(&FaultRandom(false), STORE, 1),
        Err(SessionError::RandomCollision)
    ));
    struct PartialRandom(CountingRandom);
    impl SessionRandom for PartialRandom {
        fn fill(&self, output: &mut [u8]) -> Result<(), SessionError> {
            self.0.fill(output)?;
            if self.0.0.load(Ordering::SeqCst) == 4 {
                Err(SessionError::RandomUnavailable)
            } else {
                Ok(())
            }
        }
    }
    let partial = PartialRandom(CountingRandom::default());
    assert!(matches!(
        generate(&partial, STORE, 1),
        Err(SessionError::RandomUnavailable)
    ));
    assert_eq!(partial.0.0.load(Ordering::SeqCst), 4);
}

#[test]
fn debug_is_redacted_and_same_totp_secret_cannot_hide_behind_another_reference() {
    let generated = generate(&CountingRandom::default(), STORE, 1).unwrap();
    assert_eq!(
        format!("{:?}", generated.codes),
        "RecoveryCodes([REDACTED])"
    );
    assert_eq!(
        format!("{:?}", generated.codes.codes()[0]),
        "RecoveryCode([REDACTED])"
    );
    let old = crate::session::TotpSeed::new("old".into(), [9; 20]).unwrap();
    let renamed = crate::session::TotpSeed::new("new".into(), [9; 20]).unwrap();
    let changed = crate::session::TotpSeed::new("new".into(), [8; 20]).unwrap();
    assert!(renamed.matches_binding(old.reference(), &old.fingerprint()));
    assert!(!changed.matches_binding(old.reference(), &old.fingerprint()));
}
