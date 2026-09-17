//! 当前凭据摘要只用于内部撤权复核，既不接受跨类型摘要，也不会把旧摘要恢复为有效。
use sha2::{Digest as _, Sha256};
use std::{fs, os::unix::fs::PermissionsExt};
use studiolo_supply::AgentStore;

#[test]
fn credential_fingerprint_tracks_type_revocation_rotation_and_store_failure() {
    let temp = tempfile::TempDir::new_in(fs::canonicalize("/tmp").unwrap()).unwrap();
    let data = temp.path().join("data");
    fs::create_dir(&data).unwrap();
    fs::set_permissions(&data, fs::Permissions::from_mode(0o700)).unwrap();
    let store = AgentStore::open(&data).unwrap();
    let first = store.issue_type("claude", "2026-09-12T00:00:00Z").unwrap();
    let digest: String = Sha256::digest(first.token.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    assert!(store.credential_is_current("claude", &digest).unwrap());
    assert!(!store.credential_is_current("codex", &digest).unwrap());
    assert!(!store.credential_is_current("claude", "invalid").unwrap());
    store.revoke_type("claude", "2026-09-12T00:00:01Z").unwrap();
    assert!(!store.credential_is_current("claude", &digest).unwrap());
    let second = store.issue_type("claude", "2026-09-12T00:00:02Z").unwrap();
    let second_digest: String = Sha256::digest(second.token.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    assert!(!store.credential_is_current("claude", &digest).unwrap());
    assert!(
        store
            .credential_is_current("claude", &second_digest)
            .unwrap()
    );
    fs::set_permissions(&data, fs::Permissions::from_mode(0o750)).unwrap();
    assert!(
        store
            .credential_is_current("claude", &second_digest)
            .is_err()
    );
}
