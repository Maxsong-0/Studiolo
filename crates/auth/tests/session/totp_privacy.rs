//! Debug、真实SQLite及审计只保留允许的引用/元数据，不保留原种子或输入验证码。

use super::*;
use studiolo_audit_ledger::{AuditFilter, AuditLedger};

#[test]
fn debug_database_and_audit_never_contain_raw_seed_or_codes() {
    let h = TotpHarness::enrolled(3600, 7200);
    let owner = identity();
    let issued = h.h.manager.issue(&owner).unwrap();
    h.totp
        .step_up(&request(&owner, &issued, 1), code(NEXT))
        .unwrap();
    let seed_text = std::str::from_utf8(&SEED).unwrap();
    let codes = [code_text(START), code_text(NEXT)];
    let debug = format!("{:?} {:?}", seed(), code(NEXT));
    assert!(!debug.contains(seed_text));
    assert!(!debug.contains(SEED_REF));
    for code in &codes {
        assert!(!debug.contains(code));
    }
    let records = h.h.audit.query(&AuditFilter::default()).unwrap();
    let audit = serde_json::to_string(&records).unwrap();
    assert!(!audit.contains(seed_text));
    assert!(!audit.contains(SEED_REF));
    for code in &codes {
        assert!(!audit.contains(code));
    }
    // 检查真实 Auth 文件及 WAL，不仅读当前行；不存在的 WAL 不属于失败。
    for name in ["auth.sqlite3", "auth.sqlite3-wal"] {
        let path = h.h.root.join(name);
        if path.exists() {
            let bytes = std::fs::read(path).unwrap();
            assert!(!bytes.windows(SEED.len()).any(|part| part == SEED));
            for code in &codes {
                assert!(
                    !bytes
                        .windows(code.len())
                        .any(|part| part == code.as_bytes())
                );
            }
        }
    }
}
