//! 超长路径在创建父目录或持久 Preparing/Prepared 之前拒绝，不能把写者留在未决槽。
use std::fs;
use studiolo_core::{Revision, VaultPath};
use studiolo_vault_broker::{
    BlobSource, BrokerError, FileOperationLimits, FileOperationMetadata, FileOperationRequest,
    FileOperationScopeHash, FsVaultBroker, Mutation, MutationActor, MutationKind, MutationMetadata,
    Precondition, StreamLimits, StreamMutation, VaultReader, VaultWriter,
};

fn path(value: impl AsRef<str>) -> VaultPath {
    VaultPath::parse(value.as_ref()).unwrap()
}
fn actor() -> MutationActor {
    MutationActor::Obsidian
}

struct Fixture {
    temp: tempfile::TempDir,
    broker: FsVaultBroker,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir_in(fs::canonicalize(std::env::temp_dir()).unwrap()).unwrap();
        let broker =
            FsVaultBroker::open(&temp.path().join("vault"), &temp.path().join("data")).unwrap();
        Self { temp, broker }
    }
    fn state(&self) -> Vec<u8> {
        fs::read(self.temp.path().join("data/writer-state.json")).unwrap()
    }
    fn assert_unchanged(&self, before: &[u8]) {
        assert_eq!(self.state(), before);
        assert!(!self.broker.write_status().unwrap().unresolved);
        assert!(!self.temp.path().join("vault/missing").exists());
    }
}

fn invalid_paths() -> Vec<VaultPath> {
    vec![
        path(format!("{}.md", "a".repeat(253))),
        path(format!("missing/{}.md", "界".repeat(86))),
        path(format!("missing/{}/note.md", "b".repeat(256))),
    ]
}

#[test]
fn inline_write_rejects_every_overlong_component_without_leaving_a_prepared_record() {
    for invalid in invalid_paths() {
        let mut f = Fixture::new();
        let before = f.state();
        assert!(matches!(
            f.broker
                .mutate(Mutation::create(invalid, b"body".to_vec(), "oversize")),
            Err(BrokerError::WriterLimitExceeded)
        ));
        f.assert_unchanged(&before);
        drop(f.broker);
        let mut reopened =
            FsVaultBroker::open_existing(&f.temp.path().join("vault"), &f.temp.path().join("data"))
                .unwrap();
        assert!(
            reopened
                .mutate(Mutation::create(
                    path("valid.md"),
                    b"valid".to_vec(),
                    "valid"
                ))
                .is_ok()
        );
    }
}

#[test]
fn exactly_255_utf8_bytes_can_be_installed() {
    let mut f = Fixture::new();
    let exact = path("界".repeat(85));
    assert_eq!(exact.as_str().len(), 255);
    f.broker
        .mutate(Mutation::create(
            exact.clone(),
            b"body".to_vec(),
            "boundary",
        ))
        .unwrap();
    assert_eq!(f.broker.read(&exact).unwrap().content, b"body");
}

#[test]
fn streamed_write_rejects_before_staging_or_creating_missing_parents() {
    for invalid in invalid_paths() {
        let mut f = Fixture::new();
        let source_root = f.temp.path().join("source");
        fs::create_dir(&source_root).unwrap();
        fs::write(source_root.join("input.bin"), b"source").unwrap();
        let source = BlobSource::open(&source_root, path("input.bin")).unwrap();
        let limits = StreamLimits::new(1024).unwrap();
        let request = StreamMutation {
            path: invalid,
            precondition: Precondition::MustNotExist,
            content: source.identity(limits).unwrap(),
            idempotency_key: "stream-overlong".into(),
        };
        let before = f.state();
        let result = f.broker.prepare_stream_mutation(
            request,
            &source,
            limits,
            MutationMetadata::new(actor(), MutationKind::Replace).unwrap(),
        );
        assert!(matches!(result, Err(BrokerError::WriterLimitExceeded)));
        f.assert_unchanged(&before);
    }
}

#[test]
fn delete_restore_and_both_rename_paths_reject_before_preparing() {
    let mut f = Fixture::new();
    f.broker
        .enable_file_operations(FileOperationLimits {
            effective_object_limit: 1024 * 1024,
            max_tombstones: 64,
            retained_bytes_limit: 8 * 1024 * 1024,
            minimum_free_bytes: 1024,
        })
        .unwrap();
    f.broker.enable_file_rename().unwrap();
    let metadata = FileOperationMetadata::new(
        actor(),
        FileOperationScopeHash::new("1".repeat(64)).unwrap(),
        1,
    )
    .unwrap();
    let expected = Revision::from_bytes([1; 32]);
    let before = f.state();
    for invalid in invalid_paths() {
        let requests = [
            FileOperationRequest::Delete {
                path: invalid.clone(),
                expected,
                idempotency_key: "delete".into(),
            },
            FileOperationRequest::RestoreTombstone {
                path: invalid.clone(),
                tombstone_id: "a".repeat(64),
                idempotency_key: "restore".into(),
            },
            FileOperationRequest::Rename {
                from: path("from.md"),
                expected,
                to: invalid.clone(),
                idempotency_key: "rename-target".into(),
            },
            FileOperationRequest::Rename {
                from: invalid,
                expected,
                to: path("to.md"),
                idempotency_key: "rename-source".into(),
            },
        ];
        for request in requests {
            assert!(matches!(
                f.broker.prepare_file_operation(request, metadata.clone()),
                Err(BrokerError::WriterLimitExceeded)
            ));
            f.assert_unchanged(&before);
        }
    }
}
