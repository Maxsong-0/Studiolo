//! SQLite-backed runtime authority owned exclusively by TaskEngine.
//!
//! Writes require `&mut self`; a transaction only moves plain data and never
//! calls another Studiolo module.

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

const SCHEMA_VERSION: i64 = 3;

#[derive(Clone, Copy, PartialEq, Eq)]
enum TerminalKind {
    Legacy,
    BoardReview,
    BoardStopped,
}
impl TerminalKind {
    fn outcome(self) -> Option<Value> {
        (self == Self::BoardStopped).then(|| Value::String("stopped".into()))
    }
    fn required_phase(self) -> &'static str {
        if self == Self::BoardStopped {
            "cancel_requested"
        } else {
            "running"
        }
    }
}

/// Control-plane persistence failures.
#[derive(Debug, thiserror::Error)]
pub enum DbError {
    /// SQLite rejected an operation.
    #[error("control database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// JSON serialization or parsing failed.
    #[error("control database JSON error: {0}")]
    Json(#[from] serde_json::Error),
    /// The database is already bound to another Vault.
    #[error("control database is bound to {expected}, not {actual}")]
    VaultBindingMismatch {
        /// Persisted binding.
        expected: VaultUuid,
        /// Requested binding.
        actual: VaultUuid,
    },
    /// A referenced record does not exist.
    #[error("control record not found: {0}")]
    NotFound(&'static str),
    /// The transition is invalid from the current durable state.
    #[error("invalid transition from {from} via {operation}")]
    InvalidTransition {
        /// Current state.
        from: String,
        /// Requested operation.
        operation: &'static str,
    },
    /// 已有终态赢家（含已冻结提交计划的 committing）；属正常业务拒绝，调用方按变体而非状态名判断。
    #[error("attempt already holds a terminal outcome ({phase:?}) for {operation}")]
    AlreadyTerminal {
        /// 持有终态结果的持久阶段。
        phase: AttemptPhase,
        /// 被拒绝的操作名。
        operation: &'static str,
    },
    /// 数据库由更新版本的 schema 写入，本二进制不得静默降级运行。
    #[error("control db schema version {found} is newer than supported version {supported}")]
    UnsupportedSchema {
        /// `schema_migrations` 中记录的最高版本。
        found: i64,
        /// 本二进制支持的最高版本。
        supported: i64,
    },
    /// Attempt identity or fence did not match.
    #[error("attempt fence mismatch")]
    FenceMismatch,
    /// Active work owns one or more requested paths.
    #[error("paths held by {by_instance}: {paths:?}")]
    PathsHeld {
        #[doc = "Instance owning the conflicting claim."]
        by_instance: String,
        #[doc = "Requested paths intersecting the active claim."]
        paths: Vec<String>,
    },
    /// A machine-checkable blackboard rule rejected the operation.
    #[error("blackboard rejected: {0}")]
    BoardRejected(&'static str),
    /// A stable request id was used with different inputs.
    #[error("idempotency request mismatch")]
    IdempotencyMismatch,
    /// A durable value was malformed.
    #[error("corrupt control record: {0}")]
    Corrupt(String),
}

/// Authoritative task lifecycle state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    /// Human-authored definition not yet queued.
    Draft,
    /// Eligible for claim.
    Queued,
    /// Reserved by a worker while run preparation is completing.
    Claimed,
    /// An attempt currently owns the task.
    Running,
    /// Terminal effects are reserved and must roll forward.
    Committing,
    /// Cancellation is requested while runner isolation completes.
    CancelRequested,
    /// Artifacts await human review.
    NeedsReview,
    /// Human revision is required before another run.
    NeedsRevision,
    /// Inputs or policy changed and require confirmation.
    NeedsReconfirm,
    /// Human review approved the result.
    Approved,
    /// Successful terminal state.
    Succeeded,
    /// Failed terminal state.
    Failed,
    /// Cancelled terminal state.
    Cancelled,
    /// Isolated after restart.
    Abandoned,
}

impl TaskStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Queued => "queued",
            Self::Claimed => "claimed",
            Self::Running => "running",
            Self::Committing => "committing",
            Self::CancelRequested => "cancel_requested",
            Self::NeedsReview => "needs_review",
            Self::NeedsRevision => "needs_revision",
            Self::NeedsReconfirm => "needs_reconfirm",
            Self::Approved => "approved",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Abandoned => "abandoned",
        }
    }

    fn parse(value: &str) -> Result<Self, DbError> {
        match value {
            "draft" => Ok(Self::Draft),
            "queued" => Ok(Self::Queued),
            "claimed" => Ok(Self::Claimed),
            "running" => Ok(Self::Running),
            "committing" => Ok(Self::Committing),
            "cancel_requested" => Ok(Self::CancelRequested),
            "needs_review" => Ok(Self::NeedsReview),
            "needs_revision" => Ok(Self::NeedsRevision),
            "needs_reconfirm" => Ok(Self::NeedsReconfirm),
            "approved" => Ok(Self::Approved),
            "succeeded" => Ok(Self::Succeeded),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            "abandoned" => Ok(Self::Abandoned),
            other => Err(DbError::Corrupt(format!("unknown task status {other}"))),
        }
    }
}

/// Durable attempt phase.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttemptPhase {
    /// Claimed but not started.
    Claimed,
    /// Runner work may be in flight.
    Running,
    /// Capability revoked; runner isolation pending.
    CancelRequested,
    /// Terminal effects must roll forward idempotently.
    Committing,
    /// Successful terminal attempt.
    Completed,
    /// Failed terminal attempt.
    Failed,
    /// Cancelled terminal attempt.
    Cancelled,
    /// Restart-isolated attempt.
    Abandoned,
}

impl AttemptPhase {
    fn as_str(self) -> &'static str {
        match self {
            Self::Claimed => "claimed",
            Self::Running => "running",
            Self::CancelRequested => "cancel_requested",
            Self::Committing => "committing",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Abandoned => "abandoned",
        }
    }

    fn parse(value: &str) -> Result<Self, DbError> {
        match value {
            "claimed" => Ok(Self::Claimed),
            "running" => Ok(Self::Running),
            "cancel_requested" => Ok(Self::CancelRequested),
            "committing" => Ok(Self::Committing),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            "abandoned" => Ok(Self::Abandoned),
            other => Err(DbError::Corrupt(format!("unknown attempt phase {other}"))),
        }
    }
}

/// Persisted task projection and human-authored definition.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TaskRecord {
    /// Stable task identifier.
    pub task_id: TaskId,
    /// Human-authored definition; runtime fields live in columns.
    pub definition_json: Value,
    /// Runtime status.
    pub status: TaskStatus,
    /// Active attempt, if any.
    pub active_attempt: Option<AttemptId>,
    /// Most recent terminal attempt, if any.
    pub last_attempt: Option<AttemptId>,
    /// Immutable input snapshot projected for the current or last attempt.
    pub input_snapshot: Option<String>,
    /// Original creation time for deterministic task-page rendering.
    pub created_at_ms: i64,
}

/// Metadata retained for one snapshotted source.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SnapshotEntryRecord {
    /// Logical Vault path.
    pub path: String,
    /// Exact source revision.
    pub revision: String,
    /// Effective labels, never source content.
    pub effective_label_json: Value,
}

/// Metadata-only immutable snapshot manifest.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SnapshotRecord {
    /// Content-addressed snapshot identifier.
    pub snapshot_id: String,
    /// Bound Vault identity.
    pub vault_uuid: VaultUuid,
    /// Source metadata.
    pub entries: Vec<SnapshotEntryRecord>,
}

/// Data durably reserved before terminal side effects.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TerminalReservation {
    /// Event and attempt identity only. Historical v1/v2 records may still require staging migration.
    pub report_json: Value,
    /// Content-plane staging pointer {id, sha256}; new inline candidates are rejected.
    pub artifacts_json: Value,
}

/// Durable attempt record.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AttemptRecord {
    /// Attempt identifier.
    pub attempt_id: AttemptId,
    /// Owning task.
    pub task_id: TaskId,
    /// Monotonic fencing token.
    pub fence: Fence,
    /// Current phase.
    pub phase: AttemptPhase,
    /// Worker identity.
    pub worker_id: String,
    /// Immutable input snapshot.
    pub snapshot_id: String,
    /// Lease deadline as Unix milliseconds.
    pub lease_until_ms: i64,
    /// PID-safe runner reference, if attached.
    pub runner_ref_json: Option<Value>,
    /// Terminal roll-forward input, if reserved.
    pub terminal: Option<TerminalReservation>,
}

/// One pending task-page projection.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OutboxRecord {
    /// Monotonic outbox sequence.
    pub sequence: i64,
    /// Owning task.
    pub task_id: TaskId,
    /// Projection payload.
    pub payload_json: Value,
}

/// A finalized terminal transition whose authoritative audit outcome is pending.
#[derive(Clone, Debug, PartialEq)]
pub struct PendingTerminalAudit {
    /// Immutable attempt and original report reservation.
    pub attempt: AttemptRecord,
    /// Status committed by T2, independent of later task changes.
    pub status: TaskStatus,
    /// Broker receipt committed in the same transaction.
    pub receipt_json: Value,
}

/// Non-cloneable, single-writer runtime authority.
pub struct ControlDb {
    connection: Connection,
    vault_uuid: VaultUuid,
}

impl ControlDb {
    /// Opens, migrates, configures, and binds a file database.
    pub fn open(path: impl AsRef<Path>, vault_uuid: VaultUuid) -> Result<Self, DbError> {
        Self::from_connection(Connection::open(path)?, vault_uuid)
    }

    /// Opens a faithful in-memory database for tests.
    pub fn open_in_memory(vault_uuid: VaultUuid) -> Result<Self, DbError> {
        Self::from_connection(Connection::open_in_memory()?, vault_uuid)
    }

    fn from_connection(connection: Connection, vault_uuid: VaultUuid) -> Result<Self, DbError> {
        // 这些设置只影响当前连接；新版库判定前不切换持久 journal mode。
        connection.execute_batch(
            "PRAGMA synchronous=FULL;
             PRAGMA foreign_keys=ON;
             PRAGMA busy_timeout=5000;
             PRAGMA trusted_schema=OFF;",
        )?;
        schema_guard::require_supported_version(&connection)?;
        let mut db = Self {
            connection,
            vault_uuid,
        };
        db.migrate()?;
        db.connection.execute_batch("PRAGMA journal_mode=WAL;")?;
        db.bind_vault(vault_uuid)?;
        Ok(db)
    }

    fn migrate(&mut self) -> Result<(), DbError> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        // 在写事务取得后再次核对，不能以事务外的旧观察代替迁移准入。
        schema_guard::require_supported_version(&tx)?;
        tx.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_migrations (
                 version INTEGER PRIMARY KEY, applied_at_ms INTEGER NOT NULL);",
        )?;
        tx.execute_batch(
            "CREATE TABLE IF NOT EXISTS vault_binding (
                 singleton INTEGER PRIMARY KEY CHECK (singleton = 1), vault_uuid TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS fence_counter (
                 singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
                 next_fence INTEGER NOT NULL CHECK (next_fence > 0));
             INSERT OR IGNORE INTO fence_counter(singleton, next_fence) VALUES (1, 1);
             CREATE TABLE IF NOT EXISTS tasks (
                 task_id TEXT PRIMARY KEY, definition_json TEXT NOT NULL, status TEXT NOT NULL,
                 active_attempt TEXT, last_attempt TEXT, input_snapshot TEXT,
                 created_at_ms INTEGER NOT NULL, updated_at_ms INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS snapshots (
                 snapshot_id TEXT PRIMARY KEY, vault_uuid TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS snapshot_entries (
                 snapshot_id TEXT NOT NULL REFERENCES snapshots(snapshot_id) ON DELETE CASCADE,
                 path TEXT NOT NULL, revision TEXT NOT NULL, effective_label_json TEXT NOT NULL,
                 PRIMARY KEY(snapshot_id, path));
             CREATE TABLE IF NOT EXISTS attempts (
                 attempt_id TEXT PRIMARY KEY, task_id TEXT NOT NULL REFERENCES tasks(task_id),
                 fence INTEGER NOT NULL UNIQUE CHECK (fence > 0), phase TEXT NOT NULL,
                 worker_id TEXT NOT NULL, snapshot_id TEXT NOT NULL REFERENCES snapshots(snapshot_id),
                 lease_until_ms INTEGER NOT NULL, runner_ref_json TEXT, terminal_json TEXT,
                 created_at_ms INTEGER NOT NULL, updated_at_ms INTEGER NOT NULL);
             CREATE UNIQUE INDEX IF NOT EXISTS one_active_attempt_per_task ON attempts(task_id)
                 WHERE phase IN ('claimed', 'running', 'cancel_requested', 'committing');
             CREATE TABLE IF NOT EXISTS processed_events (
                 event_id TEXT PRIMARY KEY, result_json TEXT NOT NULL, processed_at_ms INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS projection_outbox (
                 sequence INTEGER PRIMARY KEY AUTOINCREMENT, event_key TEXT NOT NULL UNIQUE,
                 task_id TEXT NOT NULL REFERENCES tasks(task_id), payload_json TEXT NOT NULL,
                 delivered_at_ms INTEGER);
             CREATE TABLE IF NOT EXISTS artifact_commits (
                 attempt_id TEXT PRIMARY KEY REFERENCES attempts(attempt_id),
                 receipt_json TEXT NOT NULL, committed_at_ms INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS terminal_audit_outbox (
                 attempt_id TEXT PRIMARY KEY REFERENCES artifact_commits(attempt_id),
                 status TEXT NOT NULL, delivered_at_ms INTEGER);
             CREATE INDEX IF NOT EXISTS pending_terminal_audits
                 ON terminal_audit_outbox(attempt_id) WHERE delivered_at_ms IS NULL;",
        )?;
        let has_input_snapshot = {
            let mut columns = tx.prepare("PRAGMA table_info(tasks)")?;
            let names = columns
                .query_map([], |row| row.get::<_, String>(1))?
                .collect::<Result<Vec<_>, _>>()?;
            names.iter().any(|name| name == "input_snapshot")
        };
        if !has_input_snapshot {
            tx.execute_batch(
                "ALTER TABLE tasks ADD COLUMN input_snapshot TEXT;
                UPDATE tasks SET input_snapshot = (
                    SELECT snapshot_id FROM attempts
                    WHERE attempt_id = COALESCE(tasks.active_attempt, tasks.last_attempt)
                );",
            )?;
        }
        tx.execute(
            "INSERT OR IGNORE INTO schema_migrations(version, applied_at_ms) VALUES (1, 0)",
            [],
        )?;
        let previous_version: i64 =
            tx.query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
                row.get(0)
            })?;
        if previous_version < 2 {
            // v1 finalized before acknowledging audit durability. A one-time migration
            // repair is safe because the ledger deduplicates the original event/phase.
            tx.execute(
                "INSERT OR IGNORE INTO terminal_audit_outbox(attempt_id, status)
                 SELECT a.attempt_id, t.status FROM attempts a
                 JOIN artifact_commits c USING(attempt_id) JOIN tasks t USING(task_id)
                 WHERE a.phase IN ('completed', 'failed', 'cancelled') AND a.terminal_json IS NOT NULL",
                [],
            )?;
        }
        board::migrate(&tx)?;
        tx.execute(
            "INSERT OR IGNORE INTO schema_migrations(version, applied_at_ms) VALUES (2, 0)",
            [],
        )?;
        tx.execute(
            "INSERT OR IGNORE INTO schema_migrations(version, applied_at_ms) VALUES (?1, 0)",
            [SCHEMA_VERSION],
        )?;
        tx.commit()?;
        Ok(())
    }

    fn bind_vault(&mut self, requested: VaultUuid) -> Result<(), DbError> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let stored: Option<String> = tx
            .query_row(
                "SELECT vault_uuid FROM vault_binding WHERE singleton = 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        match stored {
            None => {
                tx.execute(
                    "INSERT INTO vault_binding(singleton, vault_uuid) VALUES (1, ?1)",
                    [requested.to_string()],
                )?;
            }
            Some(value) => {
                let expected = parse_vault_uuid(&value)?;
                if expected != requested {
                    return Err(DbError::VaultBindingMismatch {
                        expected,
                        actual: requested,
                    });
                }
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Returns the latest applied schema version.
    pub fn schema_version(&self) -> Result<i64, DbError> {
        Ok(self.connection.query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
            [],
            |row| row.get(0),
        )?)
    }

    /// Returns the durable Vault binding.
    pub fn bound_vault_uuid(&self) -> Result<VaultUuid, DbError> {
        let value: String = self.connection.query_row(
            "SELECT vault_uuid FROM vault_binding WHERE singleton = 1",
            [],
            |row| row.get(0),
        )?;
        parse_vault_uuid(&value)
    }

    /// Reads an allow-listed runtime pragma for diagnostics.
    pub fn pragma(&self, name: &str) -> Result<String, DbError> {
        if !matches!(name, "journal_mode" | "synchronous" | "foreign_keys") {
            return Err(DbError::Corrupt("unsupported pragma query".to_owned()));
        }
        let value: rusqlite::types::Value =
            self.connection
                .query_row(&format!("PRAGMA {name}"), [], |row| row.get(0))?;
        match value {
            rusqlite::types::Value::Integer(value) => Ok(value.to_string()),
            rusqlite::types::Value::Text(value) => Ok(value),
            _ => Err(DbError::Corrupt("unexpected pragma value".to_owned())),
        }
    }

    /// Inserts a human-authored task definition in `draft` state.
    pub fn create_task(
        &mut self,
        task_id: TaskId,
        definition_json: &Value,
        now_ms: i64,
    ) -> Result<(), DbError> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT INTO tasks(task_id, definition_json, status, created_at_ms, updated_at_ms)
             VALUES (?1, ?2, 'draft', ?3, ?3)",
            params![
                task_id.to_string(),
                serde_json::to_string(definition_json)?,
                now_ms
            ],
        )?;
        enqueue_projection(&tx, task_id, &format!("create:{task_id}"))?;
        tx.commit()?;
        Ok(())
    }

    /// Moves a draft task into the claim queue.
    pub fn queue_task(&mut self, task_id: TaskId, now_ms: i64) -> Result<(), DbError> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = tx.execute(
            "UPDATE tasks SET status = 'queued', updated_at_ms = ?2
             WHERE task_id = ?1 AND status = 'draft'",
            params![task_id.to_string(), now_ms],
        )?;
        if changed == 1 {
            enqueue_projection(&tx, task_id, &format!("queue:{task_id}"))?;
            tx.commit()?;
            Ok(())
        } else {
            drop(tx);
            Err(self.task_transition_error(task_id, "queue")?)
        }
    }

    /// Queues a draft with the immutable snapshot confirmed by its Run command.
    pub fn queue_with_snapshot(
        &mut self,
        task_id: TaskId,
        snapshot: &SnapshotRecord,
        now_ms: i64,
    ) -> Result<(), DbError> {
        if snapshot.vault_uuid != self.vault_uuid {
            return Err(DbError::VaultBindingMismatch {
                expected: self.vault_uuid,
                actual: snapshot.vault_uuid,
            });
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        persist_snapshot(&tx, snapshot)?;
        let changed = tx.execute(
            "UPDATE tasks SET status = 'queued', input_snapshot = ?2, updated_at_ms = ?3
             WHERE task_id = ?1 AND status = 'draft'",
            params![task_id.to_string(), &snapshot.snapshot_id, now_ms],
        )?;
        if changed == 0 {
            drop(tx);
            return Err(self.task_transition_error(task_id, "queue")?);
        }
        enqueue_projection(&tx, task_id, &format!("queue:{task_id}"))?;
        tx.commit()?;
        Ok(())
    }

    /// Removes a queued task from claim eligibility after its confirmed inputs change.
    pub fn require_reconfirmation(&mut self, task_id: TaskId, now_ms: i64) -> Result<(), DbError> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = tx.execute(
            "UPDATE tasks SET status = 'needs_reconfirm', updated_at_ms = ?2
             WHERE task_id = ?1 AND status = 'queued'",
            params![task_id.to_string(), now_ms],
        )?;
        if changed == 0 {
            drop(tx);
            return Err(self.task_transition_error(task_id, "require_reconfirmation")?);
        }
        enqueue_projection(&tx, task_id, &format!("reconfirm:{task_id}"))?;
        tx.commit()?;
        Ok(())
    }

    /// Returns the oldest queued task without claiming it.
    pub fn next_queued(&self) -> Result<Option<TaskRecord>, DbError> {
        let id: Option<String> = self
            .connection
            .query_row(
                "SELECT task_id FROM tasks WHERE status = 'queued'
                 ORDER BY updated_at_ms, task_id LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        match id {
            Some(value) => self.task(parse_task_id(&value)?),
            None => Ok(None),
        }
    }

    /// Looks up one task.
    pub fn task(&self, task_id: TaskId) -> Result<Option<TaskRecord>, DbError> {
        type TaskTuple = (
            String,
            String,
            Option<String>,
            Option<String>,
            Option<String>,
            i64,
        );
        let raw: Option<TaskTuple> = self
            .connection
            .query_row(
                "SELECT definition_json, status, active_attempt, last_attempt, input_snapshot, created_at_ms
                 FROM tasks WHERE task_id = ?1",
                [task_id.to_string()],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                },
            )
            .optional()?;
        raw.map(
            |(definition, status, active, last, input_snapshot, created_at_ms)| {
                Ok(TaskRecord {
                    task_id,
                    definition_json: serde_json::from_str(&definition)?,
                    status: TaskStatus::parse(&status)?,
                    active_attempt: active.as_deref().map(parse_attempt_id).transpose()?,
                    last_attempt: last.as_deref().map(parse_attempt_id).transpose()?,
                    input_snapshot,
                    created_at_ms,
                })
            },
        )
        .transpose()
    }

    /// Atomically creates the sole active attempt and stores snapshot metadata.
    pub fn claim(
        &mut self,
        task_id: TaskId,
        worker_id: &str,
        snapshot: SnapshotRecord,
        now_ms: i64,
        lease_until_ms: i64,
    ) -> Result<AttemptRecord, DbError> {
        if worker_id.is_empty()
            || lease_until_ms <= now_ms
            || snapshot.vault_uuid != self.vault_uuid
        {
            return Err(DbError::InvalidTransition {
                from: "invalid claim input".to_owned(),
                operation: "claim",
            });
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let state: Option<(String, Option<String>)> = tx
            .query_row(
                "SELECT status, input_snapshot FROM tasks WHERE task_id = ?1",
                [task_id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        match state {
            Some((status, Some(pinned)))
                if status == "queued" && pinned != snapshot.snapshot_id =>
            {
                return Err(DbError::Corrupt(
                    "claim snapshot differs from confirmed Run snapshot".to_owned(),
                ));
            }
            Some((status, _)) if status == "queued" => {}
            Some((other, _)) => {
                return Err(DbError::InvalidTransition {
                    from: other,
                    operation: "claim",
                });
            }
            None => return Err(DbError::NotFound("task")),
        }
        board::require_paths_free(&tx, task_id)?;
        lifecycle::require_retry_ready(&tx, task_id, now_ms)?;
        persist_snapshot(&tx, &snapshot)?;
        let fence_value: i64 = tx.query_row(
            "SELECT next_fence FROM fence_counter WHERE singleton = 1",
            [],
            |row| row.get(0),
        )?;
        tx.execute(
            "UPDATE fence_counter SET next_fence = next_fence + 1 WHERE singleton = 1",
            [],
        )?;
        let fence = Fence::new(
            u64::try_from(fence_value)
                .map_err(|_| DbError::Corrupt("invalid fence counter".to_owned()))?,
        )
        .ok_or_else(|| DbError::Corrupt("zero fence".to_owned()))?;
        let attempt_id = AttemptId::new();
        tx.execute(
            "INSERT INTO attempts(
                 attempt_id, task_id, fence, phase, worker_id, snapshot_id,
                 lease_until_ms, created_at_ms, updated_at_ms)
             VALUES (?1, ?2, ?3, 'running', ?4, ?5, ?6, ?7, ?7)",
            params![
                attempt_id.to_string(),
                task_id.to_string(),
                fence_value,
                worker_id,
                &snapshot.snapshot_id,
                lease_until_ms,
                now_ms
            ],
        )?;
        tx.execute(
            "UPDATE tasks SET status = 'running', active_attempt = ?2,
                 input_snapshot = ?3, updated_at_ms = ?4
             WHERE task_id = ?1",
            params![
                task_id.to_string(),
                attempt_id.to_string(),
                &snapshot.snapshot_id,
                now_ms
            ],
        )?;
        tx.execute(
            "INSERT INTO attempt_progress(attempt_id,last_progress_ms) VALUES(?1,?2)",
            params![attempt_id.to_string(), now_ms],
        )?;
        runtime_audit::enqueue(
            &tx,
            &format!("claim:{attempt_id}"),
            "task.claim",
            serde_json::json!({"task_id":task_id,"attempt_id":attempt_id,"fence":fence.get(),"by_instance":worker_id}),
        )?;
        if tx.query_row(
            "SELECT COUNT(*) FROM board_tasks WHERE task_id=?1 AND handoff_json IS NOT NULL",
            [task_id.to_string()],
            |r| r.get::<_, i64>(0),
        )? != 0
        {
            runtime_audit::enqueue(
                &tx,
                &format!("handoff-accepted:{attempt_id}"),
                "handoff.accepted",
                serde_json::json!({"task_id":task_id,"attempt_id":attempt_id,"by_instance":worker_id}),
            )?;
        }
        enqueue_projection(&tx, task_id, &format!("claim:{attempt_id}"))?;
        tx.commit()?;
        Ok(AttemptRecord {
            attempt_id,
            task_id,
            fence,
            phase: AttemptPhase::Running,
            worker_id: worker_id.to_owned(),
            snapshot_id: snapshot.snapshot_id,
            lease_until_ms,
            runner_ref_json: None,
            terminal: None,
        })
    }

    /// Attaches a PID-safe runner reference after start.
    pub fn attach_runner(
        &mut self,
        task_id: TaskId,
        attempt_id: AttemptId,
        fence: Fence,
        runner_ref_json: &Value,
        now_ms: i64,
    ) -> Result<(), DbError> {
        let changed = self.connection.execute(
            "UPDATE attempts SET runner_ref_json = ?4, updated_at_ms = ?5
             WHERE task_id = ?1 AND attempt_id = ?2 AND fence = ?3 AND phase = 'running' AND lease_until_ms > ?5",
            params![
                task_id.to_string(),
                attempt_id.to_string(),
                fence_i64(fence)?,
                serde_json::to_string(runner_ref_json)?,
                now_ms
            ],
        )?;
        if changed == 1 {
            Ok(())
        } else {
            Err(DbError::FenceMismatch)
        }
    }

    /// Extends a live attempt lease after verifying its guard.
    pub fn heartbeat(
        &mut self,
        task_id: TaskId,
        attempt_id: AttemptId,
        fence: Fence,
        lease_until_ms: i64,
        now_ms: i64,
    ) -> Result<(), DbError> {
        if lease_until_ms <= now_ms {
            return Err(DbError::FenceMismatch);
        }
        let changed = self.connection.execute(
            "UPDATE attempts SET lease_until_ms = ?4, updated_at_ms = ?5
             WHERE task_id = ?1 AND attempt_id = ?2 AND fence = ?3 AND phase = 'running' AND lease_until_ms > ?5",
            params![
                task_id.to_string(),
                attempt_id.to_string(),
                fence_i64(fence)?,
                lease_until_ms,
                now_ms
            ],
        )?;
        if changed == 1 {
            Ok(())
        } else {
            Err(DbError::FenceMismatch)
        }
    }

    /// Immediately revokes a queued or running task's active capability.
    pub fn cancel_task(&mut self, task_id: TaskId, now_ms: i64) -> Result<(), DbError> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let state: Option<(String, Option<String>)> = tx
            .query_row(
                "SELECT status, active_attempt FROM tasks WHERE task_id = ?1",
                [task_id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        match state {
            Some((status, _)) if status == "draft" || status == "queued" => {
                board::require_children_terminal(&tx, task_id)?;
                if tx.query_row(
                    "SELECT COUNT(*) FROM board_tasks WHERE task_id=?1",
                    [task_id.to_string()],
                    |r| r.get::<_, i64>(0),
                )? != 0
                {
                    return Err(DbError::BoardRejected(
                        "unclaimed board cancellation requires a structured stopped receipt after claim",
                    ));
                }
                tx.execute(
                    "UPDATE tasks SET status = 'cancelled', active_attempt = NULL,
                         updated_at_ms = ?2 WHERE task_id = ?1",
                    params![task_id.to_string(), now_ms],
                )?;
            }
            Some((status, Some(attempt))) if status == "running" => {
                tx.execute(
                    "UPDATE attempts SET phase = 'cancel_requested', updated_at_ms = ?2
                     WHERE attempt_id = ?1 AND phase = 'running'",
                    params![attempt, now_ms],
                )?;
                tx.execute(
                    "UPDATE tasks SET status = 'cancel_requested', updated_at_ms = ?2 WHERE task_id = ?1",
                    params![task_id.to_string(), now_ms],
                )?;
            }
            Some((status, _)) => {
                return Err(DbError::InvalidTransition {
                    from: status,
                    operation: "cancel",
                });
            }
            None => return Err(DbError::NotFound("task")),
        }
        tx.execute("UPDATE attempt_progress SET cancel_requested_ms=?2 WHERE attempt_id=(SELECT active_attempt FROM tasks WHERE task_id=?1)",params![task_id.to_string(),now_ms])?;
        runtime_audit::enqueue(
            &tx,
            &format!("cancel:{task_id}:{now_ms}"),
            "task.cancel_requested",
            serde_json::json!({"task_id":task_id}),
        )?;
        enqueue_projection(&tx, task_id, &format!("cancel-request:{task_id}"))?;
        tx.commit()?;
        Ok(())
    }

    /// Completes a previously requested cancellation after the runner is isolated.
    pub fn acknowledge_cancel(
        &mut self,
        task_id: TaskId,
        attempt_id: AttemptId,
        fence: Fence,
        now_ms: i64,
    ) -> Result<(), DbError> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let state: Option<(String, i64)> = tx
            .query_row(
                "SELECT phase, fence FROM attempts WHERE task_id = ?1 AND attempt_id = ?2",
                params![task_id.to_string(), attempt_id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        match state {
            Some((_, stored)) if stored != fence_i64(fence)? => return Err(DbError::FenceMismatch),
            Some((phase, _)) => match AttemptPhase::parse(&phase)? {
                AttemptPhase::Cancelled => {
                    tx.commit()?;
                    return Ok(());
                }
                AttemptPhase::CancelRequested => {}
                AttemptPhase::Claimed | AttemptPhase::Running => {
                    return Err(DbError::InvalidTransition {
                        from: phase,
                        operation: "acknowledge_cancel",
                    });
                }
                // 终态赢家已经产生：返回结构化变体，TaskEngine 不再解释状态名字符串。
                winner @ (AttemptPhase::Committing
                | AttemptPhase::Completed
                | AttemptPhase::Failed
                | AttemptPhase::Abandoned) => {
                    return Err(DbError::AlreadyTerminal {
                        phase: winner,
                        operation: "acknowledge_cancel",
                    });
                }
            },
            None => return Err(DbError::NotFound("attempt")),
        }
        board::require_children_terminal(&tx, task_id)?;
        if tx.query_row(
            "SELECT COUNT(*) FROM board_tasks WHERE task_id=?1",
            [task_id.to_string()],
            |r| r.get::<_, i64>(0),
        )? != 0
        {
            return Err(DbError::BoardRejected(
                "board cancellation requires a structured stopped receipt",
            ));
        }
        tx.execute(
            "UPDATE attempts SET phase = 'cancelled', updated_at_ms = ?2 WHERE attempt_id = ?1",
            params![attempt_id.to_string(), now_ms],
        )?;
        tx.execute(
            "UPDATE tasks SET status = 'cancelled', active_attempt = NULL, last_attempt = ?2,
                updated_at_ms = ?3 WHERE task_id = ?1",
            params![task_id.to_string(), attempt_id.to_string(), now_ms],
        )?;
        enqueue_projection(&tx, task_id, &format!("cancelled:{attempt_id}"))?;
        tx.commit()?;
        Ok(())
    }

    /// Lists revoked attempts whose cancellation still awaits runner isolation.
    pub fn cancelling_attempts(&self) -> Result<Vec<AttemptRecord>, DbError> {
        self.attempts_with_phase(AttemptPhase::CancelRequested)
    }

    /// Performs T1: reserves the terminal winner and its roll-forward input.
    pub fn reserve_terminal(
        &mut self,
        task_id: TaskId,
        attempt_id: AttemptId,
        fence: Fence,
        reservation: &TerminalReservation,
        now_ms: i64,
    ) -> Result<(), DbError> {
        self.reserve_terminal_kind(
            task_id,
            attempt_id,
            fence,
            reservation,
            now_ms,
            TerminalKind::Legacy,
        )
    }

    /// Reserves the existing T1 protocol for runner candidates requiring owner review.
    pub fn reserve_board_runner_candidates(
        &mut self,
        task_id: TaskId,
        attempt_id: AttemptId,
        fence: Fence,
        reservation: &TerminalReservation,
        now_ms: i64,
    ) -> Result<(), DbError> {
        self.reserve_terminal_kind(
            task_id,
            attempt_id,
            fence,
            reservation,
            now_ms,
            TerminalKind::BoardReview,
        )
    }

    fn reserve_terminal_kind(
        &mut self,
        task_id: TaskId,
        attempt_id: AttemptId,
        fence: Fence,
        reservation: &TerminalReservation,
        now_ms: i64,
        kind: TerminalKind,
    ) -> Result<(), DbError> {
        if !board_completion::is_pointer_reservation(reservation)
            || reservation.report_json.get("outcome") != kind.outcome().as_ref()
        {
            return Err(DbError::BoardRejected(
                "new terminal reservations must contain metadata and staging pointers only",
            ));
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let state: Option<(String, i64, Option<String>)> = tx
            .query_row(
                "SELECT phase, fence, terminal_json FROM attempts
                 WHERE task_id = ?1 AND attempt_id = ?2",
                params![task_id.to_string(), attempt_id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let encoded = serde_json::to_string(reservation)?;
        match state {
            Some((_, stored_fence, _)) if stored_fence != fence_i64(fence)? => {
                return Err(DbError::FenceMismatch);
            }
            Some((phase, _, prior))
                if phase == "committing" && prior.as_deref() == Some(encoded.as_str()) =>
            {
                tx.commit()?;
                return Ok(());
            }
            Some((phase, _, _)) if phase != kind.required_phase() => {
                return Err(DbError::InvalidTransition {
                    from: phase,
                    operation: "reserve_terminal",
                });
            }
            None => return Err(DbError::NotFound("attempt")),
            _ => {}
        }
        if kind == TerminalKind::BoardStopped {
            stopped_candidates::require_cancel_window(&tx, task_id, attempt_id, fence, now_ms)?;
        } else {
            lifecycle::require_live(&tx, task_id, attempt_id, fence, now_ms)?;
        }
        board::require_children_terminal(&tx, task_id)?;
        let is_board = tx.query_row(
            "SELECT COUNT(*) FROM board_tasks WHERE task_id=?1",
            [task_id.to_string()],
            |r| r.get::<_, i64>(0),
        )? != 0;
        if is_board != (kind != TerminalKind::Legacy) {
            return Err(DbError::BoardRejected(
                "board candidates require the runner review protocol",
            ));
        }
        if kind != TerminalKind::Legacy {
            let adapter: String = tx.query_row(
                "SELECT json_extract(definition_json,'$.adapter') FROM tasks WHERE task_id=?1",
                [task_id.to_string()],
                |r| r.get(0),
            )?;
            if adapter == "consumer" {
                return Err(DbError::BoardRejected(
                    "consumer task cannot stage runner candidates",
                ));
            }
        }
        tx.execute(
            "UPDATE attempts SET phase = 'committing', terminal_json = ?4,
                 updated_at_ms = ?5 WHERE task_id = ?1 AND attempt_id = ?2 AND fence = ?3",
            params![
                task_id.to_string(),
                attempt_id.to_string(),
                fence_i64(fence)?,
                encoded,
                now_ms
            ],
        )?;
        tx.execute(
            "UPDATE tasks SET status = 'committing', updated_at_ms = ?2 WHERE task_id = ?1",
            params![task_id.to_string(), now_ms],
        )?;
        enqueue_projection(&tx, task_id, &format!("committing:{attempt_id}"))?;
        tx.commit()?;
        Ok(())
    }

    /// Performs T2 after audit intent and Broker effects, reserving the audit outcome.
    pub fn finalize_terminal(
        &mut self,
        task_id: TaskId,
        attempt_id: AttemptId,
        fence: Fence,
        status: TaskStatus,
        artifact_receipt: Option<&Value>,
        now_ms: i64,
    ) -> Result<(), DbError> {
        self.finalize_terminal_kind(
            task_id,
            attempt_id,
            fence,
            status,
            artifact_receipt,
            None,
            now_ms,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn finalize_terminal_kind(
        &mut self,
        task_id: TaskId,
        attempt_id: AttemptId,
        fence: Fence,
        status: TaskStatus,
        artifact_receipt: Option<&Value>,
        stopped_receipt: Option<&Value>,
        now_ms: i64,
    ) -> Result<(), DbError> {
        if !matches!(
            status,
            TaskStatus::NeedsReview
                | TaskStatus::Succeeded
                | TaskStatus::Failed
                | TaskStatus::Cancelled
        ) {
            return Err(DbError::InvalidTransition {
                from: status.as_str().to_owned(),
                operation: "finalize_terminal",
            });
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let is_board = tx.query_row(
            "SELECT COUNT(*) FROM board_tasks WHERE task_id=?1",
            [task_id.to_string()],
            |r| r.get::<_, i64>(0),
        )? != 0;
        stopped_candidates::validate_finalization(
            &tx,
            task_id,
            attempt_id,
            status,
            artifact_receipt,
            stopped_receipt,
        )?;
        if is_board && status != TaskStatus::NeedsReview && stopped_receipt.is_none() {
            return Err(DbError::BoardRejected(
                "runner candidates must await owner review",
            ));
        }
        let state: Option<(String, i64, String)> = tx
            .query_row(
                "SELECT a.phase, a.fence, t.status
                 FROM attempts a JOIN tasks t ON t.task_id = a.task_id
                 WHERE a.task_id = ?1 AND a.attempt_id = ?2",
                params![task_id.to_string(), attempt_id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let terminal_phase = match status {
            TaskStatus::Failed => AttemptPhase::Failed,
            TaskStatus::Cancelled => AttemptPhase::Cancelled,
            _ => AttemptPhase::Completed,
        };
        match state {
            Some((_, stored_fence, _)) if stored_fence != fence_i64(fence)? => {
                return Err(DbError::FenceMismatch);
            }
            Some((phase, _, task_status)) if phase == terminal_phase.as_str() => {
                let stored_receipt: Option<String> = tx
                    .query_row(
                        "SELECT receipt_json FROM artifact_commits WHERE attempt_id = ?1",
                        [attempt_id.to_string()],
                        |row| row.get(0),
                    )
                    .optional()?;
                let stored_receipt = stored_receipt
                    .map(|value| serde_json::from_str::<Value>(&value))
                    .transpose()?;
                if task_status == status.as_str() && stored_receipt.as_ref() == artifact_receipt {
                    tx.commit()?;
                    return Ok(());
                }
                return Err(DbError::InvalidTransition {
                    from: task_status,
                    operation: "finalize_terminal",
                });
            }
            Some((phase, _, _)) if phase != "committing" => {
                return Err(DbError::InvalidTransition {
                    from: phase,
                    operation: "finalize_terminal",
                });
            }
            None => return Err(DbError::NotFound("attempt")),
            _ => {}
        }
        if let Some(receipt) = stopped_receipt {
            tx.execute(
                "UPDATE board_tasks SET receipt_json=?2 WHERE task_id=?1",
                params![task_id.to_string(), serde_json::to_string(receipt)?],
            )?;
            runtime_audit::enqueue(
                &tx,
                &format!("stopped-candidates:{attempt_id}"),
                "task.stopped_candidates_committed",
                serde_json::json!({"task_id":task_id,"attempt_id":attempt_id,"fence":fence.get()}),
            )?;
        }
        if let Some(receipt) = artifact_receipt {
            tx.execute(
                "INSERT OR IGNORE INTO artifact_commits(attempt_id, receipt_json, committed_at_ms)
                 VALUES (?1, ?2, ?3)",
                params![
                    attempt_id.to_string(),
                    serde_json::to_string(receipt)?,
                    now_ms
                ],
            )?;
            tx.execute(
                "INSERT INTO terminal_audit_outbox(attempt_id, status) VALUES (?1, ?2)",
                params![attempt_id.to_string(), status.as_str()],
            )?;
        }
        tx.execute(
            "UPDATE attempts SET phase = ?4, updated_at_ms = ?5
             WHERE task_id = ?1 AND attempt_id = ?2 AND fence = ?3",
            params![
                task_id.to_string(),
                attempt_id.to_string(),
                fence_i64(fence)?,
                terminal_phase.as_str(),
                now_ms
            ],
        )?;
        tx.execute(
            "UPDATE tasks SET status = ?2, active_attempt = NULL, last_attempt = ?3,
                 updated_at_ms = ?4 WHERE task_id = ?1",
            params![
                task_id.to_string(),
                status.as_str(),
                attempt_id.to_string(),
                now_ms
            ],
        )?;
        enqueue_projection(&tx, task_id, &format!("terminal:{attempt_id}"))?;
        tx.commit()?;
        Ok(())
    }

    /// Returns whether a Gateway capability is active at this instant.
    pub fn is_attempt_active(
        &self,
        task_id: TaskId,
        attempt_id: AttemptId,
        fence: Fence,
    ) -> Result<bool, DbError> {
        self.is_attempt_active_at(
            task_id,
            attempt_id,
            fence,
            jiff::Timestamp::now().as_millisecond(),
        )
    }

    /// Marks old in-flight work abandoned; `committing` remains for roll-forward.
    pub fn abandon_inflight(&mut self, now_ms: i64) -> Result<usize, DbError> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let task_ids = {
            let mut statement = tx.prepare(
                "SELECT DISTINCT task_id FROM attempts
                 WHERE phase IN ('claimed', 'running')",
            )?;
            let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        let changed = tx.execute(
            "UPDATE attempts SET phase = 'abandoned', updated_at_ms = ?1
             WHERE phase IN ('claimed', 'running')",
            [now_ms],
        )?;
        for task_id in task_ids {
            tx.execute(
                "UPDATE tasks SET status = 'abandoned', last_attempt = active_attempt,
                     active_attempt = NULL, updated_at_ms = ?2
                 WHERE task_id = ?1 AND status != 'committing'",
                params![task_id, now_ms],
            )?;
            enqueue_projection(
                &tx,
                parse_task_id(&task_id)?,
                &format!("abandoned:{task_id}"),
            )?;
        }
        tx.commit()?;
        Ok(changed)
    }

    /// Reads a committed Broker receipt without touching canonical artifacts.
    pub fn artifact_receipt(
        &self,
        task_id: TaskId,
        attempt_id: AttemptId,
    ) -> Result<Option<Value>, DbError> {
        let receipt: Option<String> = self
            .connection
            .query_row(
                "SELECT c.receipt_json FROM artifact_commits c JOIN attempts a USING(attempt_id)
             WHERE a.task_id = ?1 AND a.attempt_id = ?2",
                params![task_id.to_string(), attempt_id.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        receipt
            .map(|value| serde_json::from_str(&value).map_err(DbError::from))
            .transpose()
    }

    /// Reads only durable terminal outcomes not acknowledged by AuditLedger.
    pub fn pending_terminal_audits(&self) -> Result<Vec<PendingTerminalAudit>, DbError> {
        let mut statement = self.connection.prepare(
            "SELECT a.task_id, o.attempt_id, o.status, c.receipt_json
             FROM terminal_audit_outbox o JOIN attempts a USING(attempt_id)
             JOIN artifact_commits c USING(attempt_id)
             WHERE o.delivered_at_ms IS NULL ORDER BY o.attempt_id",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?;
        let mut outcomes = Vec::new();
        for row in rows {
            let (task, attempt, status, receipt) = row?;
            let attempt = self
                .attempt(parse_task_id(&task)?, parse_attempt_id(&attempt)?)?
                .ok_or(DbError::NotFound("pending audit attempt"))?;
            outcomes.push(PendingTerminalAudit {
                attempt,
                status: TaskStatus::parse(&status)?,
                receipt_json: serde_json::from_str(&receipt)?,
            });
        }
        Ok(outcomes)
    }

    /// Acknowledges an outcome only after AuditLedger append succeeded.
    pub fn mark_terminal_audit_delivered(
        &mut self,
        attempt_id: AttemptId,
        now_ms: i64,
    ) -> Result<(), DbError> {
        let changed = self.connection.execute(
            "UPDATE terminal_audit_outbox SET delivered_at_ms = COALESCE(delivered_at_ms, ?2)
             WHERE attempt_id = ?1",
            params![attempt_id.to_string(), now_ms],
        )?;
        if changed == 0 {
            return Err(DbError::NotFound("pending terminal audit"));
        }
        Ok(())
    }

    /// Lists reservations that reconciliation must roll forward without a runner.
    pub fn committing_attempts(&self) -> Result<Vec<AttemptRecord>, DbError> {
        self.attempts_with_phase(AttemptPhase::Committing)
    }

    /// Lists every historical attempt in creation/id order for trusted isolation maintenance.
    /// Terminal reservations are omitted, so historical inline report bodies never escape this view.
    pub fn attempts_for_task(&self, task: TaskId) -> Result<Vec<AttemptRecord>, DbError> {
        let mut statement = self.connection.prepare(
            "SELECT attempt_id FROM attempts WHERE task_id=?1 ORDER BY created_at_ms,attempt_id",
        )?;
        let ids = statement
            .query_map([task.to_string()], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        ids.into_iter()
            .map(|id| {
                let mut attempt = self
                    .attempt(task, parse_attempt_id(&id)?)?
                    .ok_or(DbError::NotFound("historical attempt"))?;
                attempt.terminal = None;
                Ok(attempt)
            })
            .collect()
    }

    /// Looks up one attempt, including its terminal roll-forward reservation.
    pub fn attempt(
        &self,
        task_id: TaskId,
        attempt_id: AttemptId,
    ) -> Result<Option<AttemptRecord>, DbError> {
        type AttemptTuple = (
            i64,
            String,
            String,
            String,
            i64,
            Option<String>,
            Option<String>,
        );
        let row: Option<AttemptTuple> = self
            .connection
            .query_row(
                "SELECT fence, phase, worker_id, snapshot_id, lease_until_ms,
                        runner_ref_json, terminal_json
                 FROM attempts WHERE task_id = ?1 AND attempt_id = ?2",
                params![task_id.to_string(), attempt_id.to_string()],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                    ))
                },
            )
            .optional()?;
        row.map(
            |(fence, phase, worker_id, snapshot_id, lease, runner, terminal)| {
                Ok(AttemptRecord {
                    attempt_id,
                    task_id,
                    fence: Fence::new(
                        u64::try_from(fence)
                            .map_err(|_| DbError::Corrupt("invalid attempt fence".to_owned()))?,
                    )
                    .ok_or_else(|| DbError::Corrupt("zero attempt fence".to_owned()))?,
                    phase: AttemptPhase::parse(&phase)?,
                    worker_id,
                    snapshot_id,
                    lease_until_ms: lease,
                    runner_ref_json: runner
                        .map(|value| serde_json::from_str(&value))
                        .transpose()?,
                    terminal: terminal
                        .map(|value| serde_json::from_str(&value))
                        .transpose()?,
                })
            },
        )
        .transpose()
    }

    /// Loads metadata for one snapshot.
    pub fn snapshot(&self, snapshot_id: &str) -> Result<Option<SnapshotRecord>, DbError> {
        let vault: Option<String> = self
            .connection
            .query_row(
                "SELECT vault_uuid FROM snapshots WHERE snapshot_id = ?1",
                [snapshot_id],
                |row| row.get(0),
            )
            .optional()?;
        let Some(vault) = vault else {
            return Ok(None);
        };
        let mut statement = self.connection.prepare(
            "SELECT path, revision, effective_label_json FROM snapshot_entries
             WHERE snapshot_id = ?1 ORDER BY path",
        )?;
        let rows = statement.query_map([snapshot_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        let mut entries = Vec::new();
        for row in rows {
            let (path, revision, labels) = row?;
            entries.push(SnapshotEntryRecord {
                path,
                revision,
                effective_label_json: serde_json::from_str(&labels)?,
            });
        }
        Ok(Some(SnapshotRecord {
            snapshot_id: snapshot_id.to_owned(),
            vault_uuid: parse_vault_uuid(&vault)?,
            entries,
        }))
    }

    /// Returns undelivered task-page projections.
    pub fn pending_outbox(&self) -> Result<Vec<OutboxRecord>, DbError> {
        let mut statement = self.connection.prepare(
            "SELECT sequence, task_id, payload_json FROM projection_outbox
             WHERE delivered_at_ms IS NULL ORDER BY sequence",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        let mut records = Vec::new();
        for row in rows {
            let (sequence, task_id, payload) = row?;
            records.push(OutboxRecord {
                sequence,
                task_id: parse_task_id(&task_id)?,
                payload_json: serde_json::from_str(&payload)?,
            });
        }
        Ok(records)
    }

    /// Marks a projection delivered after Broker acknowledgement.
    pub fn mark_outbox_delivered(&mut self, sequence: i64, now_ms: i64) -> Result<(), DbError> {
        let changed = self.connection.execute(
            "UPDATE projection_outbox SET delivered_at_ms = ?2
             WHERE sequence = ?1 AND delivered_at_ms IS NULL",
            params![sequence, now_ms],
        )?;
        if changed == 1 {
            Ok(())
        } else {
            Err(DbError::NotFound("pending outbox record"))
        }
    }

    fn attempts_with_phase(&self, wanted: AttemptPhase) -> Result<Vec<AttemptRecord>, DbError> {
        type AttemptTuple = (
            String,
            String,
            i64,
            String,
            String,
            String,
            i64,
            Option<String>,
            Option<String>,
        );
        let mut statement = self.connection.prepare(
            "SELECT attempt_id, task_id, fence, phase, worker_id, snapshot_id,
                    lease_until_ms, runner_ref_json, terminal_json
             FROM attempts WHERE phase = ?1 ORDER BY created_at_ms, attempt_id",
        )?;
        let rows = statement.query_map([wanted.as_str()], |row| {
            Ok::<AttemptTuple, rusqlite::Error>((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
                row.get(7)?,
                row.get(8)?,
            ))
        })?;
        let mut attempts = Vec::new();
        for row in rows {
            let (
                attempt_id,
                task_id,
                fence,
                phase,
                worker_id,
                snapshot_id,
                lease,
                runner,
                terminal,
            ) = row?;
            attempts.push(AttemptRecord {
                attempt_id: parse_attempt_id(&attempt_id)?,
                task_id: parse_task_id(&task_id)?,
                fence: Fence::new(
                    u64::try_from(fence)
                        .map_err(|_| DbError::Corrupt("invalid attempt fence".to_owned()))?,
                )
                .ok_or_else(|| DbError::Corrupt("zero attempt fence".to_owned()))?,
                phase: AttemptPhase::parse(&phase)?,
                worker_id,
                snapshot_id,
                lease_until_ms: lease,
                runner_ref_json: runner
                    .map(|value| serde_json::from_str(&value))
                    .transpose()?,
                terminal: terminal
                    .map(|value| serde_json::from_str(&value))
                    .transpose()?,
            });
        }
        Ok(attempts)
    }

    fn task_transition_error(
        &self,
        task_id: TaskId,
        operation: &'static str,
    ) -> Result<DbError, DbError> {
        match self.task(task_id)? {
            Some(task) => Ok(DbError::InvalidTransition {
                from: task.status.as_str().to_owned(),
                operation,
            }),
            None => Ok(DbError::NotFound("task")),
        }
    }
}

fn enqueue_projection(
    tx: &Transaction<'_>,
    task_id: TaskId,
    event_key: &str,
) -> Result<(), DbError> {
    type ProjectionTuple = (String, Option<String>, Option<String>, Option<String>);
    let (status, active, last, snapshot): ProjectionTuple = tx.query_row(
        "SELECT status, active_attempt, last_attempt, input_snapshot FROM tasks WHERE task_id = ?1",
        [task_id.to_string()],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    let payload = serde_json::json!({
        "task_id": task_id.to_string(), "status": status,
        "active_attempt": active, "last_attempt": last, "input_snapshot": snapshot,
    });
    tx.execute(
        "INSERT OR IGNORE INTO projection_outbox(event_key, task_id, payload_json) VALUES (?1, ?2, ?3)",
        params![event_key, task_id.to_string(), serde_json::to_string(&payload)?],
    )?;
    Ok(())
}

fn persist_snapshot(tx: &Transaction<'_>, snapshot: &SnapshotRecord) -> Result<(), DbError> {
    let inserted = tx.execute(
        "INSERT OR IGNORE INTO snapshots(snapshot_id, vault_uuid) VALUES (?1, ?2)",
        params![&snapshot.snapshot_id, snapshot.vault_uuid.to_string()],
    )?;
    if inserted == 0 {
        let stored: String = tx.query_row(
            "SELECT vault_uuid FROM snapshots WHERE snapshot_id = ?1",
            [&snapshot.snapshot_id],
            |row| row.get(0),
        )?;
        if stored != snapshot.vault_uuid.to_string() {
            return Err(DbError::Corrupt("snapshot id collision".to_owned()));
        }
        let stored_entries = {
            let mut statement = tx.prepare(
                "SELECT path, revision, effective_label_json FROM snapshot_entries
                 WHERE snapshot_id = ?1 ORDER BY path",
            )?;
            let rows = statement.query_map([&snapshot.snapshot_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?;
            let mut entries = Vec::new();
            for row in rows {
                let (path, revision, labels) = row?;
                entries.push(SnapshotEntryRecord {
                    path,
                    revision,
                    effective_label_json: serde_json::from_str(&labels)?,
                });
            }
            entries
        };
        let mut requested_entries = snapshot.entries.clone();
        requested_entries.sort_by(|left, right| left.path.cmp(&right.path));
        if stored_entries != requested_entries {
            return Err(DbError::Corrupt("snapshot id collision".to_owned()));
        }
        return Ok(());
    }
    for entry in &snapshot.entries {
        tx.execute(
            "INSERT INTO snapshot_entries(snapshot_id, path, revision, effective_label_json)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                &snapshot.snapshot_id,
                &entry.path,
                &entry.revision,
                serde_json::to_string(&entry.effective_label_json)?
            ],
        )?;
    }
    Ok(())
}

fn parse_task_id(value: &str) -> Result<TaskId, DbError> {
    TaskId::from_str(value).map_err(|_| DbError::Corrupt(format!("invalid task id {value}")))
}

fn parse_attempt_id(value: &str) -> Result<AttemptId, DbError> {
    AttemptId::from_str(value).map_err(|_| DbError::Corrupt(format!("invalid attempt id {value}")))
}

fn parse_vault_uuid(value: &str) -> Result<VaultUuid, DbError> {
    VaultUuid::from_str(value).map_err(|_| DbError::Corrupt("invalid Vault UUID".to_owned()))
}

fn fence_i64(fence: Fence) -> Result<i64, DbError> {
    i64::try_from(fence.get())
        .map_err(|_| DbError::Corrupt("fence exceeds SQLite range".to_owned()))
}
