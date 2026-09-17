//! 磁盘与内存穿过相同 Interface，拒绝请求不能在 ABA 后复活。

use studiolo_core::{Revision, VaultPath};
use studiolo_vault_broker::{
    BrokerError, FsVaultBroker, MemoryVaultBroker, Mutation, VaultBroker, VaultWriter,
};

fn path() -> VaultPath {
    VaultPath::parse("note.md").unwrap()
}

fn each(mut run: impl FnMut(&mut dyn VaultBroker)) {
    run(&mut MemoryVaultBroker::new());
    let parent = std::env::temp_dir().canonicalize().unwrap();
    let temp = tempfile::Builder::new()
        .prefix("studiolo-t046-")
        .tempdir_in(parent)
        .unwrap();
    let mut disk =
        FsVaultBroker::open(&temp.path().join("vault"), &temp.path().join("data")).unwrap();
    run(&mut disk);
}

#[test]
fn rejected_cas_stays_rejected_after_expected_content_returns() {
    each(|broker| {
        let first = broker
            .mutate(Mutation::create(path(), b"A".to_vec(), "first"))
            .unwrap();
        let expected = Revision::compute(broker.vault_uuid(), &path(), b"B");
        let rejected = Mutation::replace(path(), b"C".to_vec(), expected, "rejected");
        assert!(
            matches!(broker.mutate(rejected.clone()), Err(BrokerError::RevisionConflict { actual, .. }) if actual == Some(first.revision))
        );
        broker
            .mutate(Mutation::replace(
                path(),
                b"B".to_vec(),
                first.revision,
                "other",
            ))
            .unwrap();
        assert!(
            matches!(broker.mutate(rejected), Err(BrokerError::RevisionConflict { actual, .. }) if actual == Some(first.revision))
        );
        assert_eq!(broker.read(&path()).unwrap().content, b"B");
    });
}

#[test]
fn rejected_key_cannot_be_reused_for_changed_request_fields() {
    each(|broker| {
        let first = broker
            .mutate(Mutation::create(path(), b"A".to_vec(), "first"))
            .unwrap();
        let request = Mutation::create(path(), b"B".to_vec(), "rejected");
        assert!(matches!(
            broker.mutate(request),
            Err(BrokerError::RevisionConflict { .. })
        ));
        for request in [
            Mutation::create(path(), b"C".to_vec(), "rejected"),
            Mutation::replace(path(), b"B".to_vec(), first.revision, "rejected"),
            Mutation::create(
                VaultPath::parse("different.md").unwrap(),
                b"B".to_vec(),
                "rejected",
            ),
        ] {
            assert!(matches!(
                broker.mutate(request),
                Err(BrokerError::IdempotencyConflict)
            ));
        }
    });
}

#[test]
fn successful_create_and_replace_have_distinct_fingerprints() {
    each(|broker| {
        let first = broker
            .mutate(Mutation::create(path(), b"A".to_vec(), "key"))
            .unwrap();
        assert!(matches!(
            broker.mutate(Mutation::replace(
                path(),
                b"A".to_vec(),
                first.revision,
                "key"
            )),
            Err(BrokerError::IdempotencyConflict)
        ));
        let other = broker
            .mutate(Mutation::replace(
                path(),
                b"B".to_vec(),
                first.revision,
                "second",
            ))
            .unwrap();
        assert_eq!(other.previous_revision, Some(first.revision));
        assert_eq!(
            broker
                .mutate(Mutation::create(path(), b"A".to_vec(), "key"))
                .unwrap(),
            first
        );
        assert_eq!(broker.read(&path()).unwrap().content, b"B");
    });
}

#[test]
fn request_budgets_match_before_memory_or_disk_can_write() {
    each(|broker| {
        for request in [
            Mutation::create(path(), vec![], ""),
            Mutation::create(path(), vec![], "x".repeat(4097)),
            Mutation::create(path(), vec![0; 4 * 1024 * 1024 + 1], "large"),
        ] {
            assert!(broker.mutate(request).is_err());
        }
        assert!(matches!(
            broker.read(&path()),
            Err(BrokerError::NotFound(_))
        ));
    });
}

#[test]
fn replay_query_preserves_historical_receipt_after_target_changes_or_disappears() {
    let parent = std::env::temp_dir().canonicalize().unwrap();
    let temp = tempfile::Builder::new()
        .prefix("studiolo-t046-")
        .tempdir_in(parent)
        .unwrap();
    let root = temp.path().join("vault");
    let data = temp.path().join("data");
    let mut broker = FsVaultBroker::open(&root, &data).unwrap();
    let request = Mutation::create(path(), b"A".to_vec(), "key");
    let receipt = broker.mutate(request.clone()).unwrap();
    for current in [Some(b"later".as_slice()), None] {
        if let Some(bytes) = current {
            std::fs::write(root.join("note.md"), bytes).unwrap();
        } else {
            std::fs::remove_file(root.join("note.md")).unwrap();
        }
        assert_eq!(
            broker.mutation_replay(&request).unwrap(),
            Some(receipt.clone())
        );
    }
    drop(broker);
    let reopened = FsVaultBroker::open_existing(&root, &data).unwrap();
    assert_eq!(reopened.mutation_replay(&request).unwrap(), Some(receipt));
    assert!(
        reopened
            .mutation_replay(&Mutation::create(path(), b"new".to_vec(), "unknown"))
            .unwrap()
            .is_none()
    );
    assert!(matches!(
        reopened.mutation_replay(&Mutation::create(path(), b"different".to_vec(), "key")),
        Err(BrokerError::IdempotencyConflict)
    ));
}

#[test]
fn replay_query_returns_original_rejection_and_rechecks_binding() {
    let parent = std::env::temp_dir().canonicalize().unwrap();
    let temp = tempfile::Builder::new()
        .prefix("studiolo-t046-")
        .tempdir_in(parent)
        .unwrap();
    let root = temp.path().join("vault");
    let data = temp.path().join("data");
    let mut broker = FsVaultBroker::open(&root, &data).unwrap();
    let first = broker
        .mutate(Mutation::create(path(), b"A".to_vec(), "first"))
        .unwrap();
    let rejected = Mutation::create(path(), b"B".to_vec(), "rejected");
    assert!(broker.mutate(rejected.clone()).is_err());
    std::fs::remove_file(root.join("note.md")).unwrap();
    assert!(
        matches!(broker.mutation_replay(&rejected), Err(BrokerError::RevisionConflict { actual, .. }) if actual == Some(first.revision))
    );
    let marker = root.join(".workbench/vault.json");
    let changed = root.join(".workbench/replacement.json");
    std::fs::write(&changed, std::fs::read(&marker).unwrap()).unwrap();
    std::fs::rename(changed, marker).unwrap();
    assert!(broker.mutation_replay(&rejected).is_err());
}
