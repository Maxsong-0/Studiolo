//! 由 TaskEngine 独占持有、以 SQLite 为底座的运行时权威。
//!
//! 写操作要求 `&mut self`；事务只搬运普通数据，
//! 从不调用其他 Studiolo 模块。

use std::{path::Path, str::FromStr};

use rusqlite::{Connection, OptionalExtension as _, Transaction, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use studiolo_core::{AttemptId, Fence, TaskId, VaultUuid};

mod board;
mod board_completion;
mod board_projection;
mod runner_candidates;
mod schema_guard;
mod stopped_candidates;
pub use runner_candidates::RunnerCandidateStage;
mod lifecycle;
mod runtime_audit;
pub use board::{BoardDefinition, BoardRecord};
pub use lifecycle::{AttemptProgress, LeaseReconciliation};
pub use runtime_audit::RuntimeAuditRecord;

mod attempt_queries;
mod attempts;
mod cancellation;
mod connection;
mod error;
mod migration;
mod projection_outbox;
mod snapshots;
mod tasks;
mod terminal_audit;
mod terminal_finalize;
mod terminal_reserve;
mod types;
pub use connection::ControlDb;
pub use error::DbError;
pub use types::{
    AttemptPhase, AttemptRecord, OutboxRecord, PendingTerminalAudit, SnapshotEntryRecord,
    SnapshotRecord, TaskRecord, TaskStatus, TerminalReservation,
};

// 以下内部项只在 crate 内可见：各子模块经 `use super::*` 取用，
// schema_guard 经 `crate::SCHEMA_VERSION` 取用。
use migration::SCHEMA_VERSION;
use projection_outbox::enqueue_projection;
use snapshots::persist_snapshot;
use terminal_reserve::TerminalKind;
use types::{fence_i64, parse_attempt_id, parse_task_id, parse_vault_uuid};
