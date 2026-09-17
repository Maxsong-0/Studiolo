//! 实际文件上的只拒绝 CAS：读取后的 ABA 不能把范围预检升级为写入。

use std::{fs, path::PathBuf};
use studiolo_core::{Revision, VaultPath};
use studiolo_vault_broker::{BrokerError, FsVaultBroker, Mutation, VaultReader, VaultWriter};

struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    data: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let parent = std::env::temp_dir().canonicalize().unwrap();
        let temp = tempfile::Builder::new()
            .prefix("studiolo-t046-")
            .tempdir_in(parent)
            .unwrap();
        Self {
            root: temp.path().join("vault"),
            data: temp.path().join("data"),
            _temp: temp,
        }
    }
    fn broker(&self) -> FsVaultBroker {
        FsVaultBroker::open(&self.root, &self.data).unwrap()
    }
    fn journal(&self) -> Vec<u8> {
        fs::read(self.data.join("writer-state.json")).unwrap()
    }
    fn file(&self) -> PathBuf {
        self.root.join("note.md")
    }
}
fn path() -> VaultPath {
    VaultPath::parse("note.md").unwrap()
}
fn conflict(result: Result<(), BrokerError>, actual: Option<Revision>) {
    assert!(
        matches!(result, Err(BrokerError::RevisionConflict { actual: value, .. }) if value == actual)
    );
}

#[test]
fn missing_rejection_survives_recreation_and_reopen_without_installing() {
    let f = Fixture::new();
    let mut broker = f.broker();
    let first = broker
        .mutate(Mutation::create(path(), b"A".to_vec(), "first"))
        .unwrap();
    let request = Mutation::replace(path(), b"draft".to_vec(), first.revision, "missing");
    fs::remove_file(f.file()).unwrap();
    let journal = f.journal();
    conflict(broker.check_precondition(&request), None);
    assert!(!f.file().exists());
    assert_eq!(f.journal(), journal);
    fs::write(f.file(), b"A").unwrap();
    conflict(broker.check_precondition(&request), None);
    assert!(matches!(
        broker.mutate(request.clone()),
        Err(BrokerError::RevisionConflict { actual: None, .. })
    ));
    drop(broker);
    let mut broker = FsVaultBroker::open_existing(&f.root, &f.data).unwrap();
    conflict(broker.check_precondition(&request), None);
    assert_eq!(fs::read(f.file()).unwrap(), b"A");
    assert_eq!(f.journal(), journal);
}

#[test]
fn a_matching_check_has_no_stage_history_receipt_or_filesystem_installation() {
    let f = Fixture::new();
    let mut broker = f.broker();
    let request = Mutation::create(path(), b"new".to_vec(), "check-only");
    let journal = f.journal();
    broker.check_precondition(&request).unwrap();
    assert!(!f.file().exists());
    assert!(broker.mutation_replay(&request).unwrap().is_none());
    assert_eq!(f.journal(), journal);
    for directory in [
        f.root.join(".workbench/staging"),
        f.data.join("history"),
        f.data.join("idempotency"),
    ] {
        assert_eq!(fs::read_dir(directory).unwrap().count(), 0);
    }
}

#[test]
fn changed_generated_baseline_rejection_cannot_revive_when_expected_returns() {
    let f = Fixture::new();
    let mut broker = f.broker();
    let original = b"human A\n<!-- start -->\nG1\n<!-- end -->\n";
    let observed = b"human B\n<!-- start -->\nG2\n<!-- end -->\n";
    let first = broker
        .mutate(Mutation::create(path(), original.to_vec(), "first"))
        .unwrap();
    fs::write(f.file(), observed).unwrap();
    let before = broker.read(&path()).unwrap();
    assert_ne!(before.revision, first.revision);
    let request = Mutation::replace(path(), observed.to_vec(), first.revision, "old-baseline");
    conflict(broker.check_precondition(&request), Some(before.revision));
    fs::write(f.file(), original).unwrap();
    conflict(broker.check_precondition(&request), Some(before.revision));
    assert!(matches!(
        broker.mutate(request),
        Err(BrokerError::RevisionConflict { .. })
    ));
    assert_eq!(fs::read(f.file()).unwrap(), original);
}

#[test]
fn baseline_returns_before_check_only_reports_match_and_never_installs_b() {
    let f = Fixture::new();
    let mut broker = f.broker();
    let first = broker
        .mutate(Mutation::create(path(), b"G1".to_vec(), "first"))
        .unwrap();
    fs::write(f.file(), b"G2").unwrap();
    let stale_read = broker.read(&path()).unwrap();
    assert_ne!(stale_read.revision, first.revision);
    let request = Mutation::replace(path(), stale_read.content, first.revision, "returned");
    fs::write(f.file(), b"G1").unwrap();
    let journal = f.journal();
    broker.check_precondition(&request).unwrap();
    // 此结果没有验证生成区；owner 只能刷新重读，不把旧 B 正文直接交给 mutate。
    assert_eq!(fs::read(f.file()).unwrap(), b"G1");
    assert!(broker.mutation_replay(&request).unwrap().is_none());
    assert_eq!(f.journal(), journal);
}

#[test]
fn completed_history_and_conflicting_keys_obey_original_terminal() {
    let f = Fixture::new();
    let mut broker = f.broker();
    let request = Mutation::create(path(), b"A".to_vec(), "first");
    broker.mutate(request.clone()).unwrap();
    fs::remove_file(f.file()).unwrap();
    let journal = f.journal();
    broker.check_precondition(&request).unwrap();
    assert!(!f.file().exists());
    assert_eq!(f.journal(), journal);
    assert!(matches!(
        broker.check_precondition(&Mutation::create(path(), b"B".to_vec(), "first")),
        Err(BrokerError::IdempotencyConflict)
    ));
    let expected = Revision::compute(broker.vault_uuid(), &path(), b"A");
    let rejected = Mutation::replace(path(), b"B".to_vec(), expected, "reject");
    conflict(broker.check_precondition(&rejected), None);
    for changed in [
        Mutation::create(path(), b"B".to_vec(), "reject"),
        Mutation::replace(path(), b"C".to_vec(), expected, "reject"),
    ] {
        assert!(matches!(
            broker.check_precondition(&changed),
            Err(BrokerError::IdempotencyConflict)
        ));
    }
}

#[test]
fn budgets_and_storage_bindings_fail_before_new_terminal_is_created() {
    let f = Fixture::new();
    let mut broker = f.broker();
    let journal = f.journal();
    for request in [
        Mutation::create(path(), vec![], ""),
        Mutation::create(path(), vec![], "x".repeat(4097)),
        Mutation::create(path(), vec![0; 4 * 1024 * 1024 + 1], "large"),
        Mutation::create(
            VaultPath::parse(".workbench/private.md").unwrap(),
            vec![],
            "protected",
        ),
    ] {
        assert!(broker.check_precondition(&request).is_err());
    }
    assert_eq!(f.journal(), journal);
    let binding = f.data.join("broker-binding.json");
    let replacement = f.data.join("replacement.json");
    fs::write(&replacement, fs::read(&binding).unwrap()).unwrap();
    fs::rename(replacement, binding).unwrap();
    assert!(
        broker
            .check_precondition(&Mutation::create(path(), vec![], "bound"))
            .is_err()
    );
    assert_eq!(fs::read_dir(f.data.join("idempotency")).unwrap().count(), 0);
}
