//! 以目录能力句柄为根访问 canonical Studiolo Vault。
//!
//! 本 crate 是唯一获准修改 canonical Vault 文件的生产 crate。
//! 调用方只传入逻辑 [`VaultPath`] 值，绝不传入宿主（ambient）路径。

mod numbering;
pub use numbering::NumberingScanner;
mod artifact_stage;
pub use artifact_stage::{ArtifactStager, StagedArtifactCommit};
mod batch;
pub use batch::{
    BatchChange, BatchDiff, BatchEdit, BatchPreview, BatchReceipt, BatchRequest, BatchWriter,
    TextPatch,
};
mod blob;
mod stream;
mod sync_observation;
pub use stream::{PreparedStream, StreamMutation};
pub use sync_observation::{
    CanonicalSyncFile, CanonicalSyncObservation, CanonicalSyncPresence, CanonicalSyncSurvey,
    SyncReadRule, SyncReadSelection,
};
mod canonical;
mod capture;
mod check_precondition;
mod content_audit;
pub(crate) mod file_operation;
mod gateway_seal;
pub use gateway_seal::{GatewayCallReceipt, GatewayCallSeal, GatewayPublisher};
mod history;
#[cfg(test)]
mod l0_tests;
mod maintenance;
pub use file_operation::{
    FileEffect, FileOperationAuditCandidate, FileOperationAuditEvent, FileOperationKind,
    FileOperationLimits, FileOperationMetadata, FileOperationOutcome, FileOperationPhase,
    FileOperationPreview, FileOperationReceipt, FileOperationRequest, FileOperationScopeHash,
    FileOperationStatus, FileOperationTicket, FilePresence, TombstoneEntry, TombstonePage,
};
mod memory_write;
pub use memory_write::{MemoryEntryRequest, MemoryPreview, MemoryWriter};
mod memory_mutation;
mod memory_namespace;
mod mutation;
mod readonly;
mod recorded;
mod recovery;
mod replay_read;
mod restore;
mod writer;

pub use blob::{BlobIdentity, BlobPruneReport, BlobSource, StreamLimits};
pub use canonical::{CanonicalRead, CanonicalReadVault};
pub use capture::{CanonicalCapture, CaptureStamp};
pub use content_audit::{
    ContentAuditCandidate, ContentAuditEvent, ContentAuditOutcome, ContentAuditPhase,
    ContentAuditStatus,
};
pub use history::{
    HistoryContent, HistoryEntry, HistoryPage, HistoryPruneReport, HistoryRetention,
};
pub use maintenance::{
    MaintenanceActor, MaintenanceAuditCandidate, MaintenanceAuditEvent, MaintenanceCurrent,
    MaintenanceDecision, MaintenanceFileOperation, MaintenanceOutcome, MaintenancePending,
    MaintenanceReceipt, MaintenanceSecondaryTarget, MaintenanceStatus, MaintenanceTarget,
    MaintenanceTargetKind, MaintenanceTicket, MaintenanceUnavailableReason,
};
pub use readonly::{
    LocalReadFile, LocalReadObservation, LocalReadPresence, LocalReadSurvey, ReadBatch, ReadEntry,
    ReadEntryKind, ReadNote, ReadOnlyVault, ScanLimits,
};
pub use recorded::{
    MutationActor, MutationKind, MutationMetadata, PendingRecordedRestore, RecordedRestore,
};
pub use recovery::{
    PendingDecision, PendingMutation, ReconciliationOutcome, ReconciliationReceipt,
    StagePruneReport, WriterPhase,
};
pub use restore::{
    LineageState, MaintenanceCoverage, NewRootRebind, PhysicalId, RebindCoverage, RestoreError,
    RestoreLineage, RestoreReceipt, read_restore_lineage, rebind_new_roots,
};
pub use writer::WriterStatus;

mod broker;
mod error;
mod fs;
mod memory;
mod types;

pub use broker::{SnapshotReader, VaultBroker, VaultReader, VaultSnapshotter, VaultWriter};
pub use error::BrokerError;
pub use fs::FsVaultBroker;
pub use memory::MemoryVaultBroker;
pub use types::{
    ArtifactCandidate, ArtifactCommit, ArtifactCommitReceipt, CommittedArtifact, Mutation,
    MutationReceipt, Precondition, Snapshot, SnapshotFile, SnapshotId, SnapshotInput, VaultObject,
};

// 各子模块经 `crate::` 路径引用的内部名字，拆分后仍从 crate 根可达。
use error::memory_state_error;
use fs::collision_component_key;
use studiolo_core::{AttemptId, Fence, Revision, TaskId, VaultPath, VaultUuid};
use types::{MutationReplay, VaultIdentity, hex, sha256};
