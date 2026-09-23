//! 契约测试的共享假实现：固定时钟、可配置 owner、封存票据与清单输入；
//! Worker 侧的内存状态存储与假 restic 在 `worker` 子模块，经本模块原路径导出。
//!
//! 这些假实现只用于驱动状态与判定，绝不假装真实备份目标存在。

mod worker;

pub use worker::{MemoryStore, PickyLedger, StubProbe, StubRunner};

use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};

use studiolo_backup::checkpoint::barrier::BarrierClock;
use studiolo_backup::checkpoint::manifest::{
    AuditCoverageProof, CheckpointManifest, DeploymentEntry, ManifestInputs, PackageDigest,
    SecretReference, SecretSource,
};
use studiolo_backup::checkpoint::types::{
    OwnerRoster, SealTicket, StagingBudget, StagingMaterialization, TransferSource,
    TransferSourceKind,
};
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
        CheckpointOwner::Policy => OwnerWatermark::Policy {
            schema_version: 1,
            effective_version: 5,
            effective_hash: DIGEST.to_owned(),
            global_revoked: false,
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
