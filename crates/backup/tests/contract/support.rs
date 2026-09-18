//! 契约测试的共享假实现：固定时钟、可配置 owner、内存状态存储与假 restic。
//!
//! 这些假实现只用于驱动状态与判定，绝不假装真实备份目标存在。

use std::sync::Mutex;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};

use studiolo_audit_ledger::{
    AuditError, AuditEvent, AuditFilter, AuditLedger, AuditPhase, AuditRecord, MemoryAuditLedger,
};
use studiolo_backup::checkpoint::barrier::BarrierClock;
use studiolo_backup::checkpoint::manifest::{
    AuditCoverageProof, CheckpointManifest, DeploymentEntry, ManifestInputs, PackageDigest,
    SecretReference, SecretSource,
};
use studiolo_backup::checkpoint::types::{
    OwnerRoster, SealTicket, StagingBudget, StagingMaterialization, TransferSource,
    TransferSourceKind,
};
use studiolo_backup::store::model::{BackupStateStore, PersistedState, StoreError};
use studiolo_backup::worker::job::RemoteProbe;
use studiolo_backup::worker::restic::{
    ResticError, ResticInvocation, ResticOutcome, ResticRunner, SnapshotRef,
};
use studiolo_backup::worker::state::{JobKey, RemoteVerification};
use studiolo_core::VaultUuid;
use studiolo_core::checkpoint::{
    CheckpointError, CheckpointId, CheckpointOwner, CheckpointParticipant, DrainProof,
    FreezeBudget, FreezeReceipt, FreezeRequest, InstantMillis, OwnerAssembly, OwnerWatermark,
    SealedOwnerCheckpoint, WalCheckpointProof,
};

pub const DIGEST: &str = "ab12cd34ef56ab78cd90ef12ab34cd56ef78ab90cd12ef34ab56cd78ef90ab12";

/// 可手动推进的固定时钟。
pub struct TestClock(AtomicI64);

impl TestClock {
    pub fn new(start: i64) -> Self {
        Self(AtomicI64::new(start))
    }

    pub fn advance(&self, by: i64) {
        self.0.fetch_add(by, Ordering::SeqCst);
    }
}

impl BarrierClock for TestClock {
    fn now(&self) -> InstantMillis {
        InstantMillis::from_millis(self.0.load(Ordering::SeqCst))
    }
}

/// 完全排空的证明。
pub fn drained() -> DrainProof {
    DrainProof {
        in_flight_operations: 0,
        prepared_readable: false,
        background_delivery_pending: 0,
        local_maintenance_writers_blocked: true,
        implicit_read_writes_blocked: true,
    }
}

pub fn watermark(owner: CheckpointOwner, vault: VaultUuid, pair: VaultUuid) -> OwnerWatermark {
    match owner {
        CheckpointOwner::Broker => OwnerWatermark::Broker {
            vault_uuid: vault,
            pair_id: pair,
            committed_through: 42,
            unresolved: false,
        },
        CheckpointOwner::Control => OwnerWatermark::Control {
            schema_version: 2,
            vault_uuid: vault,
            outbox_pending: 0,
            outbox_delivered_through: 17,
        },
        CheckpointOwner::Auth => OwnerWatermark::Auth {
            schema_version: 3,
            owner_binding_digest: DIGEST.to_owned(),
            audit_anchor_offset: 91,
        },
        CheckpointOwner::Audit => OwnerWatermark::Audit {
            last_offset: 91,
            last_hash: DIGEST.to_owned(),
        },
        CheckpointOwner::Bridge => OwnerWatermark::Bridge {
            link_id: "link-1".to_owned(),
            scope_digest: DIGEST.to_owned(),
            common_base_digest: DIGEST.to_owned(),
            observation_sequence: 7,
            durable_gate_closed: true,
        },
        CheckpointOwner::AgentStore => OwnerWatermark::AgentStore {
            schema_version: 1,
            type_token_digest: DIGEST.to_owned(),
            instance_count: 2,
        },
    }
}

pub fn verified_checkpoint(
    owner: CheckpointOwner,
    vault: VaultUuid,
    pair: VaultUuid,
) -> SealedOwnerCheckpoint {
    SealedOwnerCheckpoint {
        owner,
        schema_version: 1,
        wal: WalCheckpointProof {
            busy: 0,
            log_frames: 12,
            checkpointed_frames: 12,
        },
        target_synced: true,
        parent_directory_synced: true,
        reopen_verified: true,
        source_binding_stable: true,
        watermark: watermark(owner, vault, pair),
    }
}

/// 一个可配置的 owner 参与者。
pub struct StubParticipant<'a> {
    pub owner: CheckpointOwner,
    pub assembly: OwnerAssembly,
    pub drain: DrainProof,
    pub freeze_cost: i64,
    pub clock: &'a TestClock,
    pub export: SealedOwnerCheckpoint,
    pub freeze_error: Option<CheckpointError>,
    pub thawed: AtomicUsize,
}

impl<'a> StubParticipant<'a> {
    pub fn new(
        owner: CheckpointOwner,
        clock: &'a TestClock,
        vault: VaultUuid,
        pair: VaultUuid,
    ) -> Self {
        Self {
            owner,
            assembly: OwnerAssembly::Present,
            drain: drained(),
            freeze_cost: 1,
            clock,
            export: verified_checkpoint(owner, vault, pair),
            freeze_error: None,
            thawed: AtomicUsize::new(0),
        }
    }
}

impl CheckpointParticipant for StubParticipant<'_> {
    fn owner(&self) -> CheckpointOwner {
        self.owner
    }

    fn assembly(&self) -> OwnerAssembly {
        self.assembly
    }

    fn freeze(&self, _request: &FreezeRequest) -> Result<FreezeReceipt, CheckpointError> {
        self.clock.advance(self.freeze_cost);
        if let Some(error) = self.freeze_error.clone() {
            return Err(error);
        }
        Ok(FreezeReceipt {
            owner: self.owner,
            drain: self.drain,
        })
    }

    fn export(&self, _request: &FreezeRequest) -> Result<SealedOwnerCheckpoint, CheckpointError> {
        Ok(self.export.clone())
    }

    fn thaw(&self) -> Result<(), CheckpointError> {
        self.thawed.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

pub fn request(id: &str, budget_millis: u32) -> FreezeRequest {
    FreezeRequest {
        checkpoint_id: CheckpointId::parse(id).expect("checkpoint id"),
        budget: FreezeBudget::new(budget_millis).expect("freeze budget"),
    }
}

pub fn transfer_source(root: &std::path::Path) -> TransferSource {
    TransferSource {
        root: root.to_path_buf(),
        kind: TransferSourceKind::DirectExport,
        materialization: StagingMaterialization::StreamedCopy,
        expires_at: InstantMillis::from_millis(5_000),
        budget: StagingBudget::FIXED,
    }
}

pub fn full_roster() -> OwnerRoster {
    OwnerRoster::new(
        CheckpointOwner::FREEZE_ORDER
            .iter()
            .map(|owner| (*owner, OwnerAssembly::Present)),
    )
    .expect("roster")
}

pub fn ticket(vault: VaultUuid, pair: VaultUuid, root: &std::path::Path) -> SealTicket {
    SealTicket {
        checkpoint_id: CheckpointId::parse("cp-0001").expect("checkpoint id"),
        roster: full_roster(),
        owners: CheckpointOwner::FREEZE_ORDER
            .iter()
            .map(|owner| verified_checkpoint(*owner, vault, pair))
            .collect(),
        transfer_source: transfer_source(root),
        content_cutoff: InstantMillis::from_millis(1_000),
    }
}

pub fn manifest_inputs() -> ManifestInputs {
    ManifestInputs {
        application_packages: vec![PackageDigest {
            name: "studiolo-local-server".to_owned(),
            version: "0.0.1".to_owned(),
            digest: DIGEST.to_owned(),
        }],
        cli_packages: vec![PackageDigest {
            name: "studiolo-auth-admin".to_owned(),
            version: "0.0.1".to_owned(),
            digest: DIGEST.to_owned(),
        }],
        deployment: vec![DeploymentEntry {
            unit: "studiolo-local-server.service".to_owned(),
            revision: "r-2026-09-17".to_owned(),
        }],
        secret_references: vec![SecretReference {
            logical_name: "restic_repository_password".to_owned(),
            source: SecretSource::EnvironmentVariable {
                variable: "RESTIC_PASSWORD_FILE".to_owned(),
            },
        }],
        audit_coverage: AuditCoverageProof {
            acked_events_checked: 12,
            acked_events_matched: 12,
            offset_comparison_only: false,
        },
    }
}

pub fn manifest(vault: VaultUuid, pair: VaultUuid, root: &std::path::Path) -> CheckpointManifest {
    CheckpointManifest::build(&ticket(vault, pair, root), manifest_inputs()).expect("manifest")
}

/// 内存状态存储；用来把 Worker 的持久语义与真实 IO 分开验证。
#[derive(Default)]
pub struct MemoryStore {
    state: Mutex<Option<PersistedState>>,
    manifest: Mutex<Option<CheckpointManifest>>,
    pub fail_manifest_commit: Mutex<Option<StoreError>>,
}

impl BackupStateStore for MemoryStore {
    fn load_state(&self) -> Result<Option<PersistedState>, StoreError> {
        Ok(self.state.lock().expect("state lock").clone())
    }

    fn save_state(&self, state: &PersistedState) -> Result<(), StoreError> {
        state.validate()?;
        *self.state.lock().expect("state lock") = Some(state.clone());
        Ok(())
    }

    fn commit_final_manifest(&self, manifest: &CheckpointManifest) -> Result<(), StoreError> {
        if let Some(error) = *self.fail_manifest_commit.lock().expect("fail lock") {
            return Err(error);
        }
        *self.manifest.lock().expect("manifest lock") = Some(manifest.clone());
        Ok(())
    }

    fn load_final_manifest(&self) -> Result<Option<CheckpointManifest>, StoreError> {
        Ok(self.manifest.lock().expect("manifest lock").clone())
    }
}

/// 假 restic 执行器：按脚本返回退出码或明确未验证。
pub struct StubRunner {
    pub outcome: ResticOutcome,
    pub seen: Mutex<Vec<Vec<String>>>,
}

impl StubRunner {
    pub fn exiting(status_zero: bool) -> Self {
        Self {
            outcome: ResticOutcome::Exited {
                status_zero,
                reported_objects: 3,
            },
            seen: Mutex::new(Vec::new()),
        }
    }

    pub fn unverified(reason: studiolo_backup::worker::restic::UnverifiedReason) -> Self {
        Self {
            outcome: ResticOutcome::Unverified { reason },
            seen: Mutex::new(Vec::new()),
        }
    }
}

impl ResticRunner for StubRunner {
    fn run(&self, invocation: &ResticInvocation) -> Result<ResticOutcome, ResticError> {
        self.seen
            .lock()
            .expect("seen lock")
            .push(invocation.argv().to_vec());
        Ok(self.outcome)
    }
}

/// 假远端探针：按作业键返回是否存在，以及校验结果。
pub struct StubProbe {
    pub located: Option<SnapshotRef>,
    pub verification: RemoteVerification,
    pub probed: Mutex<Vec<u64>>,
}

impl StubProbe {
    pub fn present(verified_at: i64) -> Self {
        Self {
            located: Some(SnapshotRef::parse("0a1b2c3d").expect("snapshot")),
            verification: RemoteVerification {
                objects_complete: true,
                checksum_verified: true,
                paired_checkpoint_reference: true,
                verified_at: InstantMillis::from_millis(verified_at),
            },
            probed: Mutex::new(Vec::new()),
        }
    }

    pub fn absent() -> Self {
        Self {
            located: None,
            verification: RemoteVerification {
                objects_complete: false,
                checksum_verified: false,
                paired_checkpoint_reference: false,
                verified_at: InstantMillis::from_millis(0),
            },
            probed: Mutex::new(Vec::new()),
        }
    }
}

impl RemoteProbe for StubProbe {
    fn locate(&self, key: &JobKey) -> Result<Option<SnapshotRef>, ResticError> {
        self.probed
            .lock()
            .expect("probe lock")
            .push(key.sequence.get());
        Ok(self.located.clone())
    }

    fn verify(
        &self,
        _snapshot: &SnapshotRef,
        _checkpoint_id: &CheckpointId,
    ) -> Result<RemoteVerification, ResticError> {
        Ok(self.verification)
    }
}

/// 只在指定 phase 上失败的审计存储。
pub struct PickyLedger {
    inner: MemoryAuditLedger,
    pub failing_phase: Mutex<Option<AuditPhase>>,
}

impl PickyLedger {
    pub fn new() -> Self {
        Self {
            inner: MemoryAuditLedger::default(),
            failing_phase: Mutex::new(None),
        }
    }

    pub fn fail(&self, phase: AuditPhase) {
        *self.failing_phase.lock().expect("phase lock") = Some(phase);
    }

    pub fn recover(&self) {
        *self.failing_phase.lock().expect("phase lock") = None;
    }

    pub fn records(&self) -> Vec<AuditRecord> {
        self.inner
            .query(&AuditFilter::default())
            .expect("query audit")
    }
}

impl AuditLedger for PickyLedger {
    fn append(&self, event: AuditEvent) -> Result<AuditRecord, AuditError> {
        if *self.failing_phase.lock().expect("phase lock") == Some(event.phase()) {
            return Err(AuditError::Unavailable);
        }
        self.inner.append(event)
    }

    fn query(&self, filter: &AuditFilter) -> Result<Vec<AuditRecord>, AuditError> {
        self.inner.query(filter)
    }
}
