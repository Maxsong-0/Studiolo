//! 原工作台 owner 与真实 FrozenAudit 的组合契约；三个事件族沿线上同一映射核验。
use super::{
    TIMEOUT,
    fixture::{Fixture, idle},
};
use crate::CanonicalWorkbench;
use std::{
    fs,
    os::unix::fs::PermissionsExt as _,
    sync::{Arc, atomic::Ordering},
    time::Instant,
};
use studiolo_audit_ledger::{AuditEvent, AuditLedger, SqliteAuditLedger};
use studiolo_core::VaultPath;
use studiolo_host_config::Config;
use studiolo_vault_broker::{
    FileOperationLimits, FileOperationMetadata, FileOperationRequest, FileOperationScopeHash,
    FsVaultBroker, MaintenanceActor, MaintenanceDecision, Mutation, MutationActor, MutationKind,
    MutationMetadata, VaultReader,
};

fn deadline() -> Instant {
    Instant::now() + TIMEOUT
}
fn note() -> VaultPath {
    VaultPath::parse("new.md").unwrap()
}
pub(super) fn target(f: &Fixture, name: &str) -> std::path::PathBuf {
    let path = f.root.join(name);
    fs::create_dir(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    path
}
pub(super) fn source(f: &Fixture) -> (FsVaultBroker, Arc<SqliteAuditLedger>) {
    let mut broker =
        FsVaultBroker::open_existing(&f.root.join("vault"), &f.root.join("data")).unwrap();
    let audit = Arc::new(SqliteAuditLedger::open(&f.root.join("audit")).unwrap());
    broker
        .mutate_recorded(
            Mutation::create(note(), b"original".to_vec(), "checkpoint-original"),
            MutationMetadata::new(MutationActor::Obsidian, MutationKind::Replace).unwrap(),
        )
        .unwrap();
    (broker, audit)
}
pub(super) fn flush_content(broker: &mut FsVaultBroker, audit: &SqliteAuditLedger, corrupt: u8) {
    for _ in 0..4 {
        for candidate in broker.pending_content_audit(64).unwrap() {
            let mut event = crate::editing::audit::event(candidate.event()).unwrap();
            if corrupt == 1 {
                event = AuditEvent::new(
                    event.event_id().clone(),
                    event.phase(),
                    "fixture.changed",
                    event.payload().clone(),
                );
            }
            let record = audit.append(event).unwrap();
            let hash = if corrupt == 2 {
                "a".repeat(64)
            } else {
                record.record_hash().to_hex()
            };
            broker
                .confirm_content_audit(candidate, record.offset(), &hash)
                .unwrap();
        }
    }
}
fn maintenance(broker: &mut FsVaultBroker, audit: &SqliteAuditLedger) {
    broker
        .enable_file_operations(FileOperationLimits {
            effective_object_limit: 256 * 1024 * 1024,
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
    broker
        .prepare_file_operation(
            FileOperationRequest::Delete {
                path: note(),
                expected: broker.read(&note()).unwrap().revision,
                idempotency_key: "abandoned-for-checkpoint".into(),
            },
            metadata,
        )
        .unwrap();
    for candidate in broker.pending_file_operation_audit(64).unwrap() {
        let record = audit
            .append(crate::file_operation_audit::event(candidate.event()).unwrap())
            .unwrap();
        broker
            .confirm_file_operation_audit(
                candidate,
                record.offset(),
                &record.record_hash().to_hex(),
            )
            .unwrap();
    }
    let review = broker.maintenance_inspect(64).unwrap().targets.remove(0);
    let ticket = broker
        .prepare_maintenance(
            &review.review_id,
            MaintenanceDecision::CancelPrepared,
            MaintenanceActor::current().unwrap(),
        )
        .unwrap();
    flush_maintenance(broker, audit);
    broker.commit_maintenance(ticket).unwrap();
    for candidate in broker.pending_file_operation_audit(64).unwrap() {
        let record = audit
            .append(crate::file_operation_audit::event(candidate.event()).unwrap())
            .unwrap();
        broker
            .confirm_file_operation_audit(
                candidate,
                record.offset(),
                &record.record_hash().to_hex(),
            )
            .unwrap();
    }
    flush_maintenance(broker, audit);
}
fn flush_maintenance(broker: &mut FsVaultBroker, audit: &SqliteAuditLedger) {
    for candidate in broker.pending_maintenance_audit(64).unwrap() {
        let record = audit
            .append(crate::maintenance::audit::event(candidate.event()).unwrap())
            .unwrap();
        broker
            .confirm_maintenance_audit(candidate, record.offset(), &record.record_hash().to_hex())
            .unwrap();
    }
}
pub(super) async fn start(
    f: &Fixture,
    broker: FsVaultBroker,
    audit: Arc<SqliteAuditLedger>,
) -> Arc<CanonicalWorkbench> {
    let config = Config::load(&f.config).unwrap();
    let roots = Arc::new(
        config
            .canonical_serving_roots(
                &f.root.join("vault"),
                &f.root.join("assets"),
                &f.root.join("data"),
            )
            .unwrap(),
    );
    let workbench = CanonicalWorkbench::prepare_with_audit(broker, roots, audit)
        .await
        .unwrap();
    idle(&workbench).await;
    workbench
        .core
        .probe
        .os_events
        .store(false, Ordering::Release);
    workbench
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn checkpoint_real_audit_covers_content_file_operations_and_maintenance() {
    let f = Fixture::new();
    let (mut broker, audit) = source(&f);
    flush_content(&mut broker, &audit, 0);
    maintenance(&mut broker, &audit);
    let workbench = start(&f, broker, audit.clone()).await;
    let quiet = audit.quiesce_operations(deadline()).unwrap();
    let mut frozen = workbench.freeze_checkpoint(deadline()).unwrap();
    assert!(workbench.core.owner.lock().unwrap().is_none());
    assert!(matches!(
        FsVaultBroker::open_existing(&f.root.join("vault"), &f.root.join("data")),
        Err(studiolo_vault_broker::BrokerError::WriterBusy)
    ));
    let original = audit
        .checkpoint_source()
        .unwrap()
        .freeze(deadline())
        .unwrap();
    let coverage = frozen.verify_audit(&original).unwrap();
    assert_eq!(coverage.checked(), 6);
    assert_eq!(coverage.max_offset(), 6);
    let vault = target(&f, "export-vault");
    let data = target(&f, "export-data");
    assert!(frozen.export(&vault, &data).unwrap().is_verified());
    drop(frozen);
    assert_eq!(
        workbench
            .core
            .owner
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .read(&note())
            .unwrap()
            .content,
        b"original"
    );
    drop(original);
    drop(quiet);
    workbench.shutdown().await.unwrap();
    f.available();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn checkpoint_same_id_wrong_full_event_or_hash_cannot_export() {
    for corrupt in [1, 2] {
        let f = Fixture::new();
        let (mut broker, audit) = source(&f);
        flush_content(&mut broker, &audit, corrupt);
        let workbench = start(&f, broker, audit.clone()).await;
        let quiet = audit.quiesce_operations(deadline()).unwrap();
        let mut frozen = workbench.freeze_checkpoint(deadline()).unwrap();
        let original = audit
            .checkpoint_source()
            .unwrap()
            .freeze(deadline())
            .unwrap();
        assert!(frozen.verify_audit(&original).is_err());
        assert!(
            frozen
                .export(&target(&f, "bad-vault"), &target(&f, "bad-data"))
                .is_err()
        );
        drop(frozen);
        assert!(workbench.core.owner.lock().unwrap().is_some());
        drop(original);
        drop(quiet);
        workbench.shutdown().await.unwrap();
        f.available();
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn checkpoint_failure_and_unwind_return_original_owner_without_reopening() {
    let f = Fixture::new();
    let (mut broker, audit) = source(&f);
    flush_content(&mut broker, &audit, 0);
    let workbench = start(&f, broker, audit).await;
    assert!(workbench.freeze_checkpoint(Instant::now()).is_err());
    assert!(workbench.core.owner.lock().unwrap().is_some());
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _frozen = workbench.freeze_checkpoint(deadline()).unwrap();
        panic!("模拟上层中断");
    }));
    assert!(result.is_err());
    assert!(workbench.core.owner.lock().unwrap().is_some());
    assert!(matches!(
        FsVaultBroker::open_existing(&f.root.join("vault"), &f.root.join("data")),
        Err(studiolo_vault_broker::BrokerError::WriterBusy)
    ));
    workbench.shutdown().await.unwrap();
    f.available();
}
