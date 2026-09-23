//! 跨 Control DB、VaultBroker 与 AuditLedger 的持久化任务生命周期编排。
//!
//! 本模块持有唯一可变的 [`ControlDb`] 句柄。Vault 读取与快照
//! 都在认领事务之前完成；终态产物发布使用持久的
//! T1 预留与「manifest 最后写入」的前滚。

#![forbid(unsafe_code)]

// 各职责模块都是私有的；公开项经下方 `pub use` 重导出，
// 对外路径仍是 `studiolo_task_engine::名字`。
mod board;
mod board_audit;
mod board_projection;
mod board_types;
mod engine;
mod error;
mod inputs;
mod lifecycle;
mod numbering;
mod persistent;
mod projection;
mod projection_outbox;
mod task_types;
mod terminal_commit;
mod terminal_staging;
mod transitions;
pub use board_types::*;
pub use engine::TaskEngine;
pub use error::{ProjectionFailure, TaskEngineError};
pub use persistent::PersistentTaskEngine;
pub use task_types::{
    ClaimRequest, ClaimedTask, ReconcileReport, ReportResult, RetryPolicy, TaskCommand, TaskSpec,
    TaskView,
};

// 各模块（board、lifecycle、terminal_staging 等）都经 `use super::*` 从 crate 根取名：
// 跨模块用到的私有项因此在这里重新导入，外部依赖的导入也留在根上。
use error::{map_audit, map_db};
use inputs::{snapshot_record, task_page_path};
use task_types::{decode_spec, task_view};
use terminal_commit::{AttemptIdentity, CommitPlan, decode_plan, prior_identity};

use serde::{Deserialize, Serialize};
use serde_json::json;
use studiolo_audit_ledger::{
    AuditError, AuditEvent, AuditEventId, AuditLedger, AuditPhase, AuditSubject, SanitizedPayload,
};
use studiolo_control_db::{
    AttemptPhase, AttemptRecord, ControlDb, DbError, OutboxRecord, PendingTerminalAudit,
    SnapshotEntryRecord, SnapshotRecord, TaskRecord, TaskStatus, TerminalReservation,
};
use studiolo_core::{AttemptId, Fence, TaskId, VaultPath};
use studiolo_runner_adapter::{RunInput, RunSpec, RunnerRef, RunnerReport};
use studiolo_vault_broker::{
    ArtifactCandidate, ArtifactCommit, ArtifactCommitReceipt, ArtifactStager, BrokerError,
    Mutation, Precondition, Snapshot, SnapshotId, SnapshotInput, VaultBroker,
};
