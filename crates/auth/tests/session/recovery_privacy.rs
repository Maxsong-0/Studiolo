//! 仅虚构秘密的负测；审计没有原码、OTP、seed 或引用，磁盘只保留摘要。

use super::*;
use studiolo_audit_ledger::{AuditFilter, AuditLedger};

#[test]
fn raw_recovery_codes_seeds_and_new_otp_are_absent_from_debug_database_and_audit() {
    let h = RecoveryHarness::new(3600, 7200);
    let (issued, codes) = h.ready();
    let prepared = h.prepared();
    let candidate = otp_text(&NEW, CHANGE);
    let debug = format!(
        "{:?} {:?} {:?} {:?}",
        codes,
        copy_code(&codes, 0),
        seed(NEW),
        otp(&NEW, CHANGE)
    );
    h.h.clock.set(CHANGE);
    h.recovery
        .replace_factor(
            &request(&identity(), &issued, 1),
            copy_code(&codes, 0),
            &prepared,
            otp(&NEW, CHANGE),
        )
        .unwrap();
    let records = h.h.audit.query(&AuditFilter::default()).unwrap();
    let audit = serde_json::to_string(&records).unwrap();
    for text in [&debug, &audit] {
        for code in codes.codes() {
            assert!(!text.contains(code.expose_secret()));
            assert!(!text.contains(&code.expose_secret()[23..]));
        }
        assert!(!text.contains(std::str::from_utf8(&OLD).unwrap()));
        assert!(!text.contains(std::str::from_utf8(&NEW).unwrap()));
        assert!(!text.contains("test-only-new"));
        assert!(!text.contains(&candidate));
    }
    for name in ["auth.sqlite3", "auth.sqlite3-wal"] {
        let path = h.h.root.join(name);
        if !path.exists() {
            continue;
        }
        let bytes = std::fs::read(path).unwrap();
        for code in codes.codes() {
            for text in [code.expose_secret(), &code.expose_secret()[23..]] {
                assert!(
                    !bytes
                        .windows(text.len())
                        .any(|part| part == text.as_bytes())
                );
            }
        }
        for secret in [&OLD[..], &NEW[..], candidate.as_bytes()] {
            assert!(!bytes.windows(secret.len()).any(|part| part == secret));
        }
    }
}
