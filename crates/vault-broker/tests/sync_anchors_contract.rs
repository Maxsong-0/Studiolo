//! 显式锚点初始化只改目录；原始观察缺锚点仍拒绝，所有夹具均为本测试自有。
#![cfg(unix)]

use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::PathBuf,
};
use studiolo_core::VaultPath;
use studiolo_vault_broker::{
    BlobSource, BrokerError, FsVaultBroker, Mutation, MutationActor, MutationKind,
    MutationMetadata, Precondition, StreamLimits, StreamMutation, SyncReadRule, SyncReadSelection,
    VaultReader, VaultWriter,
};

fn path(value: &str) -> VaultPath {
    VaultPath::parse(value).unwrap()
}
fn selection(folders: &[&str]) -> SyncReadSelection {
    SyncReadSelection {
        folders: folders
            .iter()
            .map(|value| SyncReadRule {
                path: path(value),
                max_bytes: 1024,
            })
            .collect(),
        files: vec![],
        excluded_prefixes: vec![],
    }
}
struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    data: PathBuf,
    broker: FsVaultBroker,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir_in(fs::canonicalize("/tmp").unwrap()).unwrap();
        let root = temp.path().join("vault");
        let data = temp.path().join("data");
        fs::create_dir(&root).unwrap();
        fs::create_dir(&data).unwrap();
        let broker = FsVaultBroker::initialize_empty(&root, &data).unwrap();
        Self {
            _temp: temp,
            root,
            data,
            broker,
        }
    }
}
#[test]
fn fresh_library_requires_explicit_anchors_then_original_capture_succeeds() {
    let mut f = Fixture::new();
    let selected = selection(&["项目/A", "_模板", "项目/B"]);
    let expected = vec![path("项目/A"), path("_模板"), path("项目/B")];
    let before = f.broker.write_status().unwrap();
    assert!(matches!(
        f.broker.capture_sync_observation(&selected, &[]),
        Err(BrokerError::NotFound(_))
    ));
    assert_eq!(f.broker.inspect_sync_anchors(&selected).unwrap(), expected);
    assert!(!f.root.join("项目").exists());
    assert_eq!(f.broker.write_status().unwrap(), before);
    assert_eq!(f.broker.prepare_sync_anchors(&selected).unwrap(), expected);
    for name in ["项目", "项目/A", "_模板", "项目/B"] {
        assert_eq!(
            fs::metadata(f.root.join(name))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }
    assert!(
        f.broker
            .capture_sync_observation(&selected, &[])
            .unwrap()
            .entries()
            .is_empty()
    );
    assert!(f.broker.inspect_sync_anchors(&selected).unwrap().is_empty());
    assert!(f.broker.prepare_sync_anchors(&selected).unwrap().is_empty());
    assert_eq!(f.broker.write_status().unwrap(), before);
}
#[test]
fn files_revisions_and_watermark_stay_unchanged_and_file_rules_do_not_create_parents() {
    let mut f = Fixture::new();
    f.broker
        .mutate(Mutation::create(
            path("notes/keep.md"),
            b"unchanged".to_vec(),
            "keep",
        ))
        .unwrap();
    let object = f.broker.read(&path("notes/keep.md")).unwrap();
    let before = f.broker.write_status().unwrap();
    let state = fs::read(f.data.join("writer-state.json")).unwrap();
    let mut selected = selection(&["notes", "_模板"]);
    selected.files.push(SyncReadRule {
        path: path("precise/config.md"),
        max_bytes: 1024,
    });
    assert_eq!(
        f.broker.inspect_sync_anchors(&selected).unwrap(),
        vec![path("_模板")]
    );
    assert_eq!(
        f.broker.prepare_sync_anchors(&selected).unwrap(),
        vec![path("_模板")]
    );
    assert!(!f.root.join("precise").exists());
    let after = f.broker.read(&path("notes/keep.md")).unwrap();
    assert_eq!(
        (after.content, after.revision),
        (object.content, object.revision)
    );
    assert_eq!(f.broker.write_status().unwrap(), before);
    assert_eq!(fs::read(f.data.join("writer-state.json")).unwrap(), state);
}
#[test]
fn every_existing_invalid_anchor_is_rejected_before_any_missing_anchor_is_created() {
    for case in [
        "file",
        "link",
        "parent-file",
        "parent-link",
        "case",
        "unicode",
        "nfd",
    ] {
        let mut f = Fixture::new();
        let target = match case {
            "file" | "parent-file" => {
                fs::write(f.root.join("bad"), b"not a directory").unwrap();
                if case == "file" { "bad" } else { "bad/child" }
            }
            "link" | "parent-link" => {
                symlink(&f.data, f.root.join("bad")).unwrap();
                if case == "link" { "bad" } else { "bad/child" }
            }
            "case" => {
                fs::create_dir(f.root.join("BAD")).unwrap();
                "bad/child"
            }
            "unicode" => {
                fs::create_dir(f.root.join("STRASSE")).unwrap();
                "Straße/child"
            }
            _ => {
                fs::create_dir(f.root.join("Cafe\u{301}")).unwrap();
                "Café/child"
            }
        };
        let selected = selection(&["new/anchor", target]);
        let before = f.broker.write_status().unwrap();
        for result in [
            f.broker.inspect_sync_anchors(&selected),
            f.broker.prepare_sync_anchors(&selected),
        ] {
            assert!(
                matches!(
                    result,
                    Err(BrokerError::UnsafeNode { .. } | BrokerError::PathCollision { .. })
                ),
                "{case}"
            );
        }
        assert!(!f.root.join("new").exists(), "{case}");
        assert_eq!(f.broker.write_status().unwrap(), before);
    }
}
#[test]
fn invalid_selection_and_overlong_missing_parent_are_rejected_without_creation() {
    let mut f = Fixture::new();
    for mut selected in [
        selection(&["new", ".hidden"]),
        selection(&["new", "new/child"]),
    ] {
        assert!(f.broker.prepare_sync_anchors(&selected).is_err());
        selected.folders[0].max_bytes = 0;
        assert!(f.broker.inspect_sync_anchors(&selected).is_err());
        assert!(!f.root.join("new").exists());
    }
    let overlong = format!("new/{}/child", "a".repeat(256));
    assert!(matches!(
        f.broker.prepare_sync_anchors(&selection(&[&overlong])),
        Err(BrokerError::WriterLimitExceeded)
    ));
    assert!(!f.root.join("new").exists());
}
#[test]
fn owner_lease_and_prepared_operation_both_block_anchor_writes() {
    let mut f = Fixture::new();
    assert!(matches!(
        FsVaultBroker::open_existing(&f.root, &f.data),
        Err(BrokerError::WriterBusy)
    ));
    let source_root = f.root.parent().unwrap().join("source");
    fs::create_dir(&source_root).unwrap();
    fs::write(source_root.join("input"), b"pending").unwrap();
    let source = BlobSource::open(&source_root, path("input")).unwrap();
    let limits = StreamLimits::new(1024).unwrap();
    let request = StreamMutation {
        path: path("pending.md"),
        precondition: Precondition::MustNotExist,
        content: source.identity(limits).unwrap(),
        idempotency_key: "pending".into(),
    };
    let ticket = f
        .broker
        .prepare_stream_mutation(
            request,
            &source,
            limits,
            MutationMetadata::new(MutationActor::Obsidian, MutationKind::Replace).unwrap(),
        )
        .unwrap();
    let before = f.broker.write_status().unwrap();
    assert!(before.unresolved && !before.read_blocked);
    let selected = selection(&["missing"]);
    assert!(matches!(
        f.broker.inspect_sync_anchors(&selected),
        Err(BrokerError::MutationUnresolved)
    ));
    assert!(matches!(
        f.broker.prepare_sync_anchors(&selected),
        Err(BrokerError::MutationUnresolved)
    ));
    assert!(!f.root.join("missing").exists());
    assert_eq!(f.broker.write_status().unwrap(), before);
    f.broker.cancel_stream_mutation(ticket).unwrap();
    assert_eq!(
        f.broker.prepare_sync_anchors(&selected).unwrap(),
        vec![path("missing")]
    );
}
#[test]
fn changed_root_binding_is_rejected_before_creation() {
    let mut f = Fixture::new();
    let moved = f.root.with_file_name("moved");
    fs::rename(&f.root, &moved).unwrap();
    fs::create_dir(&f.root).unwrap();
    let selected = selection(&["missing"]);
    assert!(matches!(
        f.broker.inspect_sync_anchors(&selected),
        Err(BrokerError::RootChanged)
    ));
    assert!(matches!(
        f.broker.prepare_sync_anchors(&selected),
        Err(BrokerError::RootChanged)
    ));
    assert!(!moved.join("missing").exists());
    assert!(!f.root.join("missing").exists());
}
/// 仅在具备用户命名空间的 Linux 夹具中执行；不操作宿主挂载或共享目录。
#[test]
#[cfg(target_os = "linux")]
#[ignore = "需显式以 unshare -Urnm 运行，只挂载当前测试自有目录"]
fn cross_device_anchor_is_rejected_before_other_anchor_creation() {
    use std::os::unix::fs::MetadataExt;
    use std::process::Command;
    assert_ne!(
        fs::read_link("/proc/self/ns/mnt").unwrap(),
        fs::read_link("/proc/1/ns/mnt").unwrap(),
        "本用例必须处于显式隔离的 mount namespace"
    );
    let mut f = Fixture::new();
    let foreign = f.root.join("foreign");
    fs::create_dir(&foreign).unwrap();
    assert!(
        Command::new("mount")
            .args(["-t", "tmpfs", "-o", "size=1m", "tmpfs"])
            .arg(&foreign)
            .status()
            .unwrap()
            .success()
    );
    let selected = selection(&["missing", "foreign/child"]);
    let inspect = f.broker.inspect_sync_anchors(&selected);
    let prepare = f.broker.prepare_sync_anchors(&selected);
    let different_device =
        fs::metadata(&foreign).unwrap().dev() != fs::metadata(&f.root).unwrap().dev();
    let missing_created = f.root.join("missing").exists();
    let child_created = foreign.join("child").exists();
    assert!(
        Command::new("umount")
            .arg(&foreign)
            .status()
            .unwrap()
            .success()
    );
    assert!(different_device);
    assert!(matches!(inspect, Err(BrokerError::UnsafeNode { .. })));
    assert!(matches!(prepare, Err(BrokerError::UnsafeNode { .. })));
    assert!(!missing_created && !child_created);
}
