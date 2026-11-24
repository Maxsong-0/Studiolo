//! 实际目录/唯一租约/复制契约；此处回调是显式模拟账本，真实 Audit 接线在组合根验证。
mod audits;
mod exported;
mod restore;
mod thawed;
use super::*;
use crate::{
    ContentAuditPhase, FileOperationLimits, FileOperationMetadata, FileOperationRequest,
    FileOperationScopeHash, MaintenanceActor, MaintenanceDecision, Mutation, MutationActor,
    MutationKind, MutationMetadata, VaultPath, VaultReader,
};
use std::{
    fs,
    os::unix::fs::{MetadataExt as _, PermissionsExt as _, symlink},
    path::PathBuf,
    time::Duration,
};
use studiolo_core::checkpoint::SealedOwnerCheckpoint;

struct Fixture {
    temp: tempfile::TempDir,
    vault: PathBuf,
    data: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir_in(fs::canonicalize(std::env::temp_dir()).unwrap()).unwrap();
        Self {
            vault: temp.path().join("vault"),
            data: temp.path().join("data"),
            temp,
        }
    }
    fn open(&self) -> FsVaultBroker {
        FsVaultBroker::open(&self.vault, &self.data).unwrap()
    }
    fn target(&self, name: &str) -> PathBuf {
        let path = self.temp.path().join(name);
        fs::create_dir(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        path
    }
}
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(30)
}
fn path() -> VaultPath {
    VaultPath::parse("note.md").unwrap()
}
fn seed(broker: &mut FsVaultBroker) {
    broker
        .mutate_recorded(
            Mutation::create(path(), "正文".as_bytes().to_vec(), "first"),
            MutationMetadata::new(MutationActor::Obsidian, MutationKind::Replace).unwrap(),
        )
        .unwrap();
}
fn flush(broker: &mut FsVaultBroker) {
    for _ in 0..4 {
        for candidate in broker.pending_content_audit(64).unwrap() {
            let offset = if candidate.event().phase == ContentAuditPhase::Intent {
                1
            } else {
                2
            };
            broker
                .confirm_content_audit(candidate, offset, &"a".repeat(64))
                .unwrap();
        }
    }
    assert!(broker.pending_content_audit(64).unwrap().is_empty());
}
/// 四样都有的写者：一笔正文、一次被维护取消的文件操作、一次完成的维护；回执都落了，没有未决。
fn full_history(fixture: &Fixture) -> FsVaultBroker {
    let mut broker = fixture.open();
    seed(&mut broker);
    flush(&mut broker);
    broker
        .enable_file_operations(FileOperationLimits {
            effective_object_limit: crate::blob::OBJECT_LIMIT,
            max_tombstones: 64,
            retained_bytes_limit: 8 * 1024 * 1024 * 1024,
            minimum_free_bytes: 1024 * 1024,
        })
        .unwrap();
    let metadata = FileOperationMetadata::new(
        MutationActor::Obsidian,
        FileOperationScopeHash::new("1".repeat(64)).unwrap(),
        1,
    )
    .unwrap();
    let original = broker
        .prepare_file_operation(
            FileOperationRequest::Delete {
                path: path(),
                expected: broker.read(&path()).unwrap().revision,
                idempotency_key: "abandoned".into(),
            },
            metadata,
        )
        .unwrap();
    broker
        .confirm_file_operation_intent(&original, 3, &"a".repeat(64))
        .unwrap();
    let target = broker.maintenance_inspect(64).unwrap().targets.remove(0);
    let ticket = broker
        .prepare_maintenance(
            &target.review_id,
            MaintenanceDecision::CancelPrepared,
            MaintenanceActor::current().unwrap(),
        )
        .unwrap();
    broker
        .confirm_maintenance_intent(&ticket, 4, &"a".repeat(64))
        .unwrap();
    broker.commit_maintenance(ticket).unwrap();
    for event in broker.pending_file_operation_audit(64).unwrap() {
        broker
            .confirm_file_operation_audit(event, 5, &"a".repeat(64))
            .unwrap();
    }
    for event in broker.pending_maintenance_audit(64).unwrap() {
        broker
            .confirm_maintenance_audit(event, 6, &"a".repeat(64))
            .unwrap();
    }
    broker
}
fn verify(_: BrokerCheckpointAudit<'_>, offset: u64, hash: &str) -> Result<(), BrokerError> {
    if offset == 0 || hash != "a".repeat(64) {
        return Err(BrokerError::CorruptWriterState);
    }
    Ok(())
}
/// 和协调器同一条路：窗口里定格，交出定格，从定格导出，在副本上逐条核，核完才有票据。
/// 用例里写者不必先解冻：导出只认定格。
fn seal(
    frozen: &mut FrozenBroker,
    vault: &Path,
    data: &Path,
) -> Result<(SealedOwnerCheckpoint, BrokerCheckpointCoverage), BrokerError> {
    frozen.capture()?;
    frozen
        .take_capture()?
        .export(vault, data, deadline())?
        .verify(deadline(), verify)
}
/// 同一套核对直接在活的写者上跑一遍：和副本上的结果对照用（以前窗口里就是这样核的）。
fn verify_live(
    broker: &FsVaultBroker,
    mut verify: impl FnMut(BrokerCheckpointAudit<'_>, u64, &str) -> Result<(), BrokerError>,
) -> Result<BrokerCheckpointCoverage, BrokerError> {
    replays::verify(broker, deadline())?;
    audit::verify(broker, deadline(), &mut verify)
}
#[test]
fn checkpoint_keeps_original_lease_and_independent_complete_copy() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    seed(&mut broker);
    let mut frozen = broker.freeze_checkpoint(deadline());
    assert!(matches!(
        frozen.watermark(),
        Err(BrokerError::MutationUnresolved)
    ));
    assert!(matches!(
        FsVaultBroker::open_existing(&fixture.vault, &fixture.data),
        Err(BrokerError::WriterBusy)
    ));
    broker = frozen.thaw();
    flush(&mut broker);
    frozen = broker.freeze_checkpoint(deadline());
    let target_vault = fixture.target("copy-vault");
    let target_data = fixture.target("copy-data");
    // 没定格就没有东西可交。
    assert!(matches!(
        frozen.take_capture(),
        Err(BrokerError::IncompleteCapture)
    ));
    let (sealed, coverage) = seal(&mut frozen, &target_vault, &target_data).unwrap();
    assert_eq!((coverage.checked(), coverage.max_offset()), (2, 2));
    assert!(sealed.is_verified());
    assert_eq!(
        fs::read(target_vault.join("note.md")).unwrap(),
        "正文".as_bytes()
    );
    assert_ne!(
        fs::metadata(target_vault.join("note.md")).unwrap().ino(),
        fs::metadata(fixture.vault.join("note.md")).unwrap().ino()
    );
    assert_eq!(
        fs::metadata(target_vault.join("note.md")).unwrap().mode() & 0o777,
        0o600
    );
    // 目标已经用过、不是空的：再导出一次拒绝，不覆盖。
    assert!(seal(&mut frozen, &target_vault, &target_data).is_err());
    let broker = frozen.thaw();
    assert_eq!(broker.read(&path()).unwrap().content, "正文".as_bytes());
    // 新根必须使用 owner 重绑定，普通打开不能把旧物理身份静默收养。
    assert!(FsVaultBroker::open_existing(&target_vault, &target_data).is_err());
    let OwnerWatermark::Broker {
        vault_uuid,
        pair_id,
        ..
    } = sealed.watermark
    else {
        panic!("wrong owner");
    };
    crate::rebind_new_roots(&crate::NewRootRebind {
        new_vault_root: &target_vault,
        new_data_root: &target_data,
        expected_vault_uuid: vault_uuid,
        expected_pair_id: pair_id,
        checkpoint_id: "checkpoint-real-copy".into(),
        restore_id: "restore-real-copy",
        parent_lineage: None,
        resume_interrupted: false,
    })
    .unwrap();
    let copy = FsVaultBroker::open_existing(&target_vault, &target_data).unwrap();
    assert_eq!(
        copy.read(&path()).unwrap().content,
        broker.read(&path()).unwrap().content
    );
}
#[test]
fn checkpoint_changed_source_and_failed_recapture_revoke_the_capture() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    seed(&mut broker);
    flush(&mut broker);
    let mut frozen = broker.freeze_checkpoint(deadline());
    frozen.capture().unwrap();
    fs::write(fixture.vault.join("note.md"), b"changed").unwrap();
    let left = fixture.target("left");
    let right = fixture.target("right");
    assert!(matches!(
        frozen
            .take_capture()
            .unwrap()
            .export(&left, &right, deadline()),
        Err(BrokerError::CaptureChanged)
    ));
    assert!(fs::read_dir(&left).unwrap().next().is_none());
    // 先前定格成了，再定格失败：先前那一份也不许再交出去。
    frozen.capture().unwrap();
    symlink("/outside", fixture.vault.join("link")).unwrap();
    assert!(frozen.capture().is_err());
    assert!(matches!(
        frozen.take_capture(),
        Err(BrokerError::IncompleteCapture)
    ));
    assert!(fs::read_dir(left).unwrap().next().is_none());
}
#[test]
fn checkpoint_deadline_links_targets_and_inode_changes_fail_closed() {
    for kind in 0..4 {
        let fixture = Fixture::new();
        let broker = fixture.open();
        let mut frozen = broker.freeze_checkpoint(if kind == 0 {
            Instant::now()
        } else {
            deadline()
        });
        match kind {
            0 => {}
            1 => {
                symlink("/outside", fixture.vault.join("link")).unwrap();
            }
            2 => {
                fs::write(fixture.vault.join("a"), b"x").unwrap();
                fs::hard_link(fixture.vault.join("a"), fixture.vault.join("b")).unwrap();
            }
            _ => {
                fs::rename(&fixture.data, fixture.temp.path().join("old-data")).unwrap();
                fs::create_dir(&fixture.data).unwrap();
            }
        }
        assert!(frozen.capture().is_err());
        assert!(frozen.take_capture().is_err());
        drop(frozen.thaw());
    }
    let fixture = Fixture::new();
    let mut frozen = fixture.open().freeze_checkpoint(deadline());
    let left = fixture.target("left");
    let right = fixture.target("right");
    fs::set_permissions(&right, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(seal(&mut frozen, &left, &right).is_err());
    fs::set_permissions(&right, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(seal(&mut frozen, &left, &left).is_err());
    assert!(fs::read_dir(left).unwrap().next().is_none());
}
