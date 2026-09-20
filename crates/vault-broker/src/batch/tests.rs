//! 故障注入检验事务边界，不把进程级夹具称为断电持久性证明。
use super::*;
mod staging;
use crate::{
    FsVaultBroker, MemoryVaultBroker, Mutation, NumberingScanner, VaultPath, VaultReader,
    VaultWriter,
};
use std::{fs, sync::atomic::Ordering};
fn path(s: &str) -> VaultPath {
    VaultPath::parse(s).unwrap()
}
fn request(id: &str) -> BatchRequest {
    BatchRequest {
        batch_id: id.into(),
        changes: vec![
            BatchChange {
                path: path("notes/A.md"),
                edit: BatchEdit::Create {
                    content: "---\ntitle: A\n---\nAlpha\n".into(),
                },
            },
            BatchChange {
                path: path("notes/B.md"),
                edit: BatchEdit::Create {
                    content: "Beta\n".into(),
                },
            },
        ],
    }
}
struct Fixture {
    _temp: tempfile::TempDir,
    vault: std::path::PathBuf,
    data: std::path::PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir_in(fs::canonicalize(std::env::temp_dir()).unwrap()).unwrap();
        Self {
            vault: temp.path().join("vault"),
            data: temp.path().join("data"),
            _temp: temp,
        }
    }
    fn open(&self) -> FsVaultBroker {
        FsVaultBroker::open(&self.vault, &self.data).unwrap()
    }
    fn reopen(&self) -> FsVaultBroker {
        FsVaultBroker::open_existing(&self.vault, &self.data).unwrap()
    }
}
fn contract(broker: &mut (impl BatchWriter + VaultReader + VaultWriter)) {
    let request = request("batch-1");
    let preview = broker.preview_batch(&request).unwrap();
    let mut tampered = preview.clone();
    tampered.diffs[0].after = "tampered".into();
    assert!(matches!(
        broker.commit_batch(request.clone(), &tampered),
        Err(crate::BrokerError::IdempotencyConflict)
    ));
    assert!(matches!(
        broker.read(&path("notes/A.md")),
        Err(crate::BrokerError::NotFound(_))
    ));
    let receipt = broker.commit_batch(request.clone(), &preview).unwrap();
    assert_eq!(receipt.mutations.len(), 2);
    assert_eq!(broker.commit_batch(request, &preview).unwrap(), receipt);
    let rolled = broker.rollback_batch("batch-1").unwrap();
    assert!(rolled.rolled_back);
    assert!(matches!(
        broker.read(&path("notes/A.md")),
        Err(crate::BrokerError::NotFound(_))
    ));
    assert!(matches!(
        broker.read(&path("notes/B.md")),
        Err(crate::BrokerError::NotFound(_))
    ));
}
#[test]
fn memory_and_disk_commit_preview_rollback_contract() {
    contract(&mut MemoryVaultBroker::new());
    contract(&mut Fixture::new().open());
}
fn cas_contract(broker: &mut (impl BatchWriter + VaultReader + VaultWriter)) {
    let request = request("cas");
    let preview = broker.preview_batch(&request).unwrap();
    broker
        .mutate(Mutation::create(
            path("notes/B.md"),
            b"Human".to_vec(),
            "human",
        ))
        .unwrap();
    assert!(matches!(
        broker.commit_batch(request, &preview),
        Err(crate::BrokerError::RevisionConflict { .. })
    ));
    assert!(matches!(
        broker.read(&path("notes/A.md")),
        Err(crate::BrokerError::NotFound(_))
    ));
    assert_eq!(broker.read(&path("notes/B.md")).unwrap().content, b"Human");
}
#[test]
fn all_cas_preflight_has_no_partial_writes() {
    cas_contract(&mut MemoryVaultBroker::new());
    cas_contract(&mut Fixture::new().open());
}
#[test]
fn exact_patch_preserves_surrounding_human_body_and_rolls_back() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    let old = broker
        .mutate(Mutation::create(
            path("note.md"),
            b"Human intro\nold\nHuman tail\n".to_vec(),
            "seed",
        ))
        .unwrap();
    let request = BatchRequest {
        batch_id: "patch".into(),
        changes: vec![BatchChange {
            path: path("note.md"),
            edit: BatchEdit::Patch {
                expected: old.revision,
                patches: vec![TextPatch {
                    before: "old".into(),
                    after: "new".into(),
                }],
            },
        }],
    };
    let preview = broker.preview_batch(&request).unwrap();
    broker.commit_batch(request, &preview).unwrap();
    assert_eq!(
        broker.read(&path("note.md")).unwrap().content,
        b"Human intro\nnew\nHuman tail\n"
    );
    broker.rollback_batch("patch").unwrap();
    assert_eq!(
        broker.read(&path("note.md")).unwrap().revision,
        old.revision
    );
}
#[test]
fn every_durable_forward_phase_recovers_as_one_batch() {
    for point in 40..=44 {
        let fixture = Fixture::new();
        let mut broker = fixture.open();
        let request = request("crash");
        let preview = broker.preview_batch(&request).unwrap();
        broker.writer.fault.store(point, Ordering::Release);
        assert!(broker.commit_batch(request.clone(), &preview).is_err());
        assert!(matches!(
            broker.read(&path("notes/A.md")),
            Err(crate::BrokerError::MutationUnresolved)
        ));
        assert!(matches!(
            broker.read_file(&path("notes/B.md"), 1024),
            Err(crate::BrokerError::MutationUnresolved)
        ));
        assert!(matches!(
            broker.mutate(Mutation::create(
                path("other.md"),
                b"other".to_vec(),
                "other"
            )),
            Err(crate::BrokerError::MutationUnresolved)
        ));
        drop(broker);
        let mut broker = fixture.reopen();
        assert!(broker.read(&path("notes/A.md")).is_ok());
        assert!(broker.read(&path("notes/B.md")).is_ok());
        assert!(!broker.commit_batch(request, &preview).unwrap().rolled_back);
    }
}
#[test]
fn interrupted_rollback_recovers_whole_batch() {
    for point in 40..=44 {
        let fixture = Fixture::new();
        let mut broker = fixture.open();
        let request = request("rollback");
        let preview = broker.preview_batch(&request).unwrap();
        broker.commit_batch(request, &preview).unwrap();
        broker.writer.fault.store(point, Ordering::Release);
        assert!(broker.rollback_batch("rollback").is_err());
        drop(broker);
        let mut broker = fixture.reopen();
        assert!(broker.rollback_batch("rollback").unwrap().rolled_back);
        assert!(matches!(
            broker.read(&path("notes/A.md")),
            Err(crate::BrokerError::NotFound(_))
        ));
        assert!(matches!(
            broker.read(&path("notes/B.md")),
            Err(crate::BrokerError::NotFound(_))
        ));
    }
}
fn rollback_conflict(broker: &mut (impl BatchWriter + VaultReader + VaultWriter)) {
    let request = request("no-overwrite");
    let preview = broker.preview_batch(&request).unwrap();
    broker.commit_batch(request, &preview).unwrap();
    let b = broker.read(&path("notes/B.md")).unwrap();
    broker
        .mutate(Mutation::replace(
            b.path.clone(),
            b"human-new".to_vec(),
            b.revision,
            "later",
        ))
        .unwrap();
    assert!(matches!(
        broker.rollback_batch("no-overwrite"),
        Err(crate::BrokerError::RevisionConflict { .. })
    ));
    assert!(broker.read(&path("notes/A.md")).is_ok());
    assert_eq!(broker.read(&b.path).unwrap().content, b"human-new");
}
#[test]
fn rollback_never_overwrites_later_human_change() {
    rollback_conflict(&mut MemoryVaultBroker::new());
    rollback_conflict(&mut Fixture::new().open());
}
#[test]
fn interrupted_batch_with_external_change_stays_closed() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    let request = request("external");
    let preview = broker.preview_batch(&request).unwrap();
    broker.writer.fault.store(42, Ordering::Release);
    assert!(broker.commit_batch(request, &preview).is_err());
    drop(broker);
    fs::write(fixture.vault.join("notes/B.md"), b"human").unwrap();
    let broker = fixture.reopen();
    assert!(broker.write_status().unwrap().read_blocked);
    assert!(matches!(
        broker.read(&path("notes/A.md")),
        Err(crate::BrokerError::MutationUnresolved)
    ));
    assert_eq!(
        fs::read(fixture.vault.join("notes/B.md")).unwrap(),
        b"human"
    );
}
#[test]
fn malformed_yaml_duplicate_and_ancestor_paths_rejected() {
    let broker = MemoryVaultBroker::new();
    let mut request = request("invalid");
    request.changes[0].edit = BatchEdit::Create {
        content: "---\ntitle: [\n---\nbody".into(),
    };
    assert!(broker.preview_batch(&request).is_err());
    request.changes[0].edit = BatchEdit::Create {
        content: "valid".into(),
    };
    request.changes[1].path = path("notes/a.md");
    assert!(broker.preview_batch(&request).is_err());
    request.changes[1].path = path("notes/A.md/child.md");
    assert!(broker.preview_batch(&request).is_err());
}
#[test]
fn numbering_uses_scoped_recursive_file_names() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    for (index, name) in [
        "tasks/T-003-task.md",
        "tasks/nested/T-010-task.md",
        "other/T-999-task.md",
        "tasks/T-100x.md",
    ]
    .iter()
    .enumerate()
    {
        broker
            .mutate(Mutation::create(
                path(name),
                b"body".to_vec(),
                format!("number-{index}"),
            ))
            .unwrap();
    }
    assert_eq!(broker.scan_number_max(&path("tasks"), "T-").unwrap(), 10);
}
