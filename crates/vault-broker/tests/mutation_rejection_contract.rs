//! 被拒绝的 `CAS` 不能把另一个请求随后的安装冒充为自己的成功。

use studiolo_core::{Revision, VaultPath};
use studiolo_vault_broker::{
    BrokerError, FsVaultBroker, MemoryVaultBroker, Mutation, VaultBroker, VaultWriter,
};
use tempfile::TempDir;

fn rejected_then_independently_installed(broker: &mut dyn VaultBroker) -> Mutation {
    let path = VaultPath::parse("notes/rejected.md").unwrap();
    let initial = broker
        .mutate(Mutation::create(path.clone(), b"A".to_vec(), "initial"))
        .unwrap();
    let wrong = Revision::compute(broker.vault_uuid(), &path, b"B");
    let rejected = Mutation::replace(path.clone(), b"C".to_vec(), wrong, "rejected-key");
    assert!(matches!(
        broker.mutate(rejected.clone()),
        Err(BrokerError::RevisionConflict { .. })
    ));
    broker
        .mutate(Mutation::replace(
            path.clone(),
            b"C".to_vec(),
            initial.revision,
            "independent-key",
        ))
        .unwrap();
    assert_eq!(broker.read(&path).unwrap().content, b"C");
    rejected
}

#[test]
fn rejected_fs_cas_stays_rejected_after_another_key_installs_the_same_content() {
    let vault = TempDir::new_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let data = TempDir::new_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let mut broker = FsVaultBroker::open(vault.path(), data.path()).unwrap();
    let request = rejected_then_independently_installed(&mut broker);
    assert!(
        matches!(
            broker.mutate(request),
            Err(BrokerError::RevisionConflict { .. })
        ),
        "a rejected request must not be upgraded to a successful receipt"
    );
}

#[test]
fn rejected_fs_cas_stays_rejected_after_reopening_both_roots() {
    let vault = TempDir::new_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let data = TempDir::new_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let mut broker = FsVaultBroker::open(vault.path(), data.path()).unwrap();
    let request = rejected_then_independently_installed(&mut broker);
    drop(broker);
    let mut reopened = FsVaultBroker::open(vault.path(), data.path()).unwrap();
    assert!(
        matches!(
            reopened.mutate(request),
            Err(BrokerError::RevisionConflict { .. })
        ),
        "reopening cannot turn another request's content into this request's proof"
    );
}

#[test]
fn memory_cas_does_not_borrow_an_independent_installation() {
    let mut broker = MemoryVaultBroker::new();
    let request = rejected_then_independently_installed(&mut broker);
    assert!(matches!(
        broker.mutate(request),
        Err(BrokerError::RevisionConflict { .. })
    ));
}
