//! Persistent task lifecycle orchestration across Control DB, VaultBroker, and AuditLedger.
//!
//! The module owns the only mutable [`ControlDb`] handle. Vault reads and snapshots
//! happen before the claim transaction; terminal artifact publication uses a durable
//! T1 reservation and manifest-last roll-forward.

#![forbid(unsafe_code)]

mod projection;

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
    ArtifactCandidate, ArtifactCommit, ArtifactCommitReceipt, BrokerError, Mutation, Precondition,
    Snapshot, SnapshotId, SnapshotInput, VaultBroker,
};

/// 一条投影事件在单轮里最多尝试的写入次数（契约 §6「每条事件最多尝试三次」）。
const MAX_PROJECTION_ATTEMPTS: usize = 3;

/// Human-authored retry declaration. M0 records this but never automatically retries.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetryPolicy {
    /// Maximum attempts an explicitly confirmed future retry policy may permit.
    pub max_attempts: u32,
    /// Minimum delay before such a future retry.
    pub backoff_seconds: u64,
    /// Explicit retryable outcome names; empty means no automatic retry.
    pub retry_on: Vec<String>,
}

/// Human-authored task definition, deliberately excluding TaskEngine projections.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskSpec {
    /// Logical role selected by the author.
    pub role: String,
    /// Runner adapter kind.
    pub adapter: String,
    /// Explicit canonical inputs to snapshot.
    pub input_paths: Vec<VaultPath>,
    /// Web destinations allowed by the task before policy intersection.
    pub allow_web: Vec<String>,
    /// Database operations allowed by the task before policy intersection.
    pub allow_db: Vec<String>,
    /// Recorded retry declaration; M0 does not execute it automatically.
    pub retry_policy: RetryPolicy,
    /// Attempt timeout in seconds.
    pub timeout: u64,
    /// Human project name.
    pub project: String,
    /// Optional parent task.
    pub parent: Option<TaskId>,
    /// Optional display title.
    pub title: Option<String>,
    /// Optional runner instruction.
    pub instruction: Option<String>,
}

impl TaskSpec {
    fn validate(&self) -> Result<(), TaskEngineError> {
        if self.role.trim().is_empty() {
            return Err(TaskEngineError::InvalidTaskSpec("role is empty"));
        }
        if self.adapter != "stub" {
            return Err(TaskEngineError::InvalidTaskSpec(
                "M0 only enables the in-process stub adapter",
            ));
        }
        if self.project.trim().is_empty() {
            return Err(TaskEngineError::InvalidTaskSpec("project is empty"));
        }
        if self.retry_policy.max_attempts == 0 {
            return Err(TaskEngineError::InvalidTaskSpec(
                "retry_policy.max_attempts is zero",
            ));
        }
        if self.timeout == 0 {
            return Err(TaskEngineError::InvalidTaskSpec("timeout is zero"));
        }
        let mut paths = self.input_paths.clone();
        paths.sort();
        if paths.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(TaskEngineError::InvalidTaskSpec(
                "input_paths contains duplicates",
            ));
        }
        Ok(())
    }
}

/// Closed task command set supported by M0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskCommand {
    /// Move a draft to the claim queue.
    Run,
    /// Revoke queued or running work.
    Cancel,
}

/// Deterministic inputs for one claim attempt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClaimRequest {
    /// Authenticated worker identity supplied by the composition root.
    pub worker_id: String,
    /// Current Unix time in milliseconds.
    pub now_ms: i64,
    /// Lease deadline in Unix milliseconds.
    pub lease_until_ms: i64,
}

/// Immutable run request returned after the claim transaction commits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClaimedTask {
    /// Snapshot identifier persisted with the attempt.
    pub snapshot_id: String,
    /// Runner-facing immutable request.
    pub run_spec: RunSpec,
}

/// Task definition and Control DB projection kept in separate fields.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskView {
    /// Stable task identity.
    pub task_id: TaskId,
    /// Human-authored definition only.
    pub spec: TaskSpec,
    /// Authoritative runtime state.
    pub status: TaskStatus,
    /// Current attempt, if any.
    pub active_attempt: Option<AttemptId>,
    /// Most recent terminal attempt, if any.
    pub last_attempt: Option<AttemptId>,
    /// Immutable input snapshot selected by the claim transaction.
    pub input_snapshot: Option<String>,
}

/// Successful terminal report result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReportResult {
    /// Final task state.
    pub status: TaskStatus,
    /// Manifest-last Broker receipt.
    pub receipt: ArtifactCommitReceipt,
    /// Whether the same durable reservation was already terminal.
    pub replayed: bool,
}

/// Work performed by one restart reconciliation pass.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReconcileReport {
    /// Running attempts revoked and abandoned first.
    pub abandoned_attempts: usize,
    /// Previously requested cancellations finalized after restart isolation.
    pub cancelled_attempts: usize,
    /// Persisted committing reservations rolled forward to review.
    pub committed_attempts: usize,
    /// Previously finalized terminal outcomes repaired from the durable audit outbox.
    pub audit_outcomes: usize,
}

/// Failures exposed by the TaskEngine seam.
#[derive(Debug, thiserror::Error)]
pub enum TaskEngineError {
    /// A human-authored definition violates the public schema.
    #[error("invalid task specification: {0}")]
    InvalidTaskSpec(&'static str),
    /// A task or attempt was absent.
    #[error("task engine record not found: {0}")]
    NotFound(&'static str),
    /// The durable state does not allow the requested command.
    #[error("invalid task transition from {from} via {operation}")]
    InvalidTransition {
        /// Durable state observed by the Control DB.
        from: String,
        /// Stable operation name.
        operation: &'static str,
    },
    /// Attempt identity or fence is no longer authoritative.
    #[error("stale attempt fence")]
    StaleFence,
    /// Another terminal transition already won.
    #[error("attempt is already terminal")]
    AlreadyTerminal,
    /// An event identity was replayed with different content.
    #[error("idempotency identity was reused with different content")]
    IdempotencyMismatch,
    /// The authoritative audit sink was unavailable; protected effects stopped.
    #[error("audit ledger is unavailable")]
    AuditUnavailable,
    /// Audit validation or persistence failed.
    #[error("audit operation failed: {0}")]
    Audit(#[source] AuditError),
    /// A Vault read or snapshot operation failed.
    #[error("Vault Broker operation failed: {0}")]
    Broker(#[source] BrokerError),
    /// A manifest-last artifact commit failed and remains reserved for reconcile.
    #[error("artifact commit failed: {0}")]
    CommitFailed(#[source] BrokerError),
    /// The Control DB failed outside a classified transition.
    #[error("Control DB operation failed: {0}")]
    ControlDb(#[source] DbError),
    /// JSON persistence failed.
    #[error("task engine JSON operation failed: {0}")]
    Json(#[from] serde_json::Error),
    /// A durable terminal reservation did not match its attempt.
    #[error("task engine integrity failure: {0}")]
    Integrity(String),
    /// A task page could not be updated without losing authored content.
    #[error("task projection failed: {0}")]
    Projection(String),
    /// Some events stayed pending because their own task pages rejected them; every other
    /// task's events in the same pass were still delivered.
    #[error("task projection incomplete: {delivered} delivered, {} task(s) left pending", failures.len())]
    ProjectionIncomplete {
        /// Events delivered during this pass.
        delivered: usize,
        /// First rejected event per quarantined task; that task's later events were skipped.
        failures: Vec<ProjectionFailure>,
    },
    /// A confirmed input changed while the task was waiting to be claimed.
    #[error("task {0} requires input reconfirmation")]
    NeedsReconfirmation(TaskId),
    /// A runner reference lacked a stable identity or its required PID/start pair.
    #[error("invalid or mismatched runner reference")]
    InvalidRunnerRef,
}

/// One projection event left pending because its own task page rejected it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectionFailure {
    /// Task whose page rejected the projection.
    pub task_id: TaskId,
    /// Outbox sequence that stays pending.
    pub sequence: i64,
    /// Stable, human-readable rejection reason.
    pub reason: String,
}

/// Synchronous, object-safe task lifecycle seam.
pub trait TaskEngine: Send {
    /// Persists a validated human-authored draft.
    fn create_task(&mut self, spec: TaskSpec, now_ms: i64) -> Result<TaskId, TaskEngineError>;

    /// Applies one closed task command and returns the resulting view.
    fn command(
        &mut self,
        task_id: TaskId,
        command: TaskCommand,
        now_ms: i64,
    ) -> Result<TaskView, TaskEngineError>;

    /// Loads the Run-confirmed snapshot and atomically claims the oldest queued task.
    fn claim(&mut self, request: ClaimRequest) -> Result<Option<ClaimedTask>, TaskEngineError>;

    /// Persists the runner's restart-safe identity under its attempt fence.
    fn attach_runner(
        &mut self,
        task_id: TaskId,
        attempt_id: AttemptId,
        fence: Fence,
        runner_ref: RunnerRef,
        now_ms: i64,
    ) -> Result<(), TaskEngineError>;

    /// Extends a live lease under its attempt fence.
    fn heartbeat(
        &mut self,
        task_id: TaskId,
        attempt_id: AttemptId,
        fence: Fence,
        lease_until_ms: i64,
        now_ms: i64,
    ) -> Result<(), TaskEngineError>;

    /// Finalizes cancellation after the composition root has stopped the runner.
    fn acknowledge_cancel(
        &mut self,
        task_id: TaskId,
        attempt_id: AttemptId,
        fence: Fence,
        now_ms: i64,
    ) -> Result<TaskView, TaskEngineError>;

    /// Replays pending protected-field projections through VaultBroker with CAS.
    fn project_pending(&mut self) -> Result<usize, TaskEngineError>;

    /// Reserves and rolls a structured runner report through audit and Broker commit.
    fn report(
        &mut self,
        event_id: &str,
        report: RunnerReport,
        now_ms: i64,
    ) -> Result<ReportResult, TaskEngineError>;

    /// Revokes old running work, then rolls forward durable committing reservations.
    fn reconcile(&mut self, now_ms: i64) -> Result<ReconcileReport, TaskEngineError>;

    /// Returns one task with definition and projection kept separate.
    fn task(&self, task_id: TaskId) -> Result<Option<TaskView>, TaskEngineError>;
}

/// Persistent M0 adapter. Its Control DB and Broker writer are exclusively owned.
pub struct PersistentTaskEngine<B, L> {
    control_db: ControlDb,
    broker: B,
    audit_ledger: L,
}

impl<B, L> PersistentTaskEngine<B, L> {
    /// Takes exclusive ownership of the Control DB and Broker writer.
    pub fn new(control_db: ControlDb, broker: B, audit_ledger: L) -> Self {
        Self {
            control_db,
            broker,
            audit_ledger,
        }
    }

    /// Read-only access for composition and restart assertions.
    #[must_use]
    pub fn control_db(&self) -> &ControlDb {
        &self.control_db
    }

    /// Read-only access to the owned Broker adapter.
    #[must_use]
    pub fn broker(&self) -> &B {
        &self.broker
    }

    /// Read-only access to the owned audit adapter.
    #[must_use]
    pub fn audit_ledger(&self) -> &L {
        &self.audit_ledger
    }

    /// Returns owned parts for an explicit composition-root handoff.
    #[must_use]
    pub fn into_parts(self) -> (ControlDb, B, L) {
        (self.control_db, self.broker, self.audit_ledger)
    }
}

impl<B, L> TaskEngine for PersistentTaskEngine<B, L>
where
    B: VaultBroker,
    L: AuditLedger,
{
    fn create_task(&mut self, spec: TaskSpec, now_ms: i64) -> Result<TaskId, TaskEngineError> {
        spec.validate()?;
        let task_id = TaskId::new();
        let definition = serde_json::to_value(spec)?;
        self.control_db
            .create_task(task_id, &definition, now_ms)
            .map_err(map_db)?;
        Ok(task_id)
    }

    fn command(
        &mut self,
        task_id: TaskId,
        command: TaskCommand,
        now_ms: i64,
    ) -> Result<TaskView, TaskEngineError> {
        let current = self
            .task(task_id)?
            .ok_or(TaskEngineError::NotFound("task"))?;
        match (command, current.status) {
            (TaskCommand::Run, TaskStatus::Draft) => {
                // Run 只要求本任务自己的投影已交付；别的任务的坏页不能挡住这条命令。
                self.project_task(task_id)?;
                let snapshot = self.capture_inputs(task_id, &current.spec)?;
                self.control_db
                    .queue_with_snapshot(task_id, &snapshot_record(&snapshot), now_ms)
                    .map_err(map_db)?;
            }
            (TaskCommand::Run, TaskStatus::Queued)
            | (TaskCommand::Cancel, TaskStatus::Cancelled | TaskStatus::CancelRequested) => {}
            (TaskCommand::Cancel, TaskStatus::Draft | TaskStatus::Queued | TaskStatus::Running) => {
                self.control_db
                    .cancel_task(task_id, now_ms)
                    .map_err(map_db)?;
            }
            (
                TaskCommand::Cancel,
                TaskStatus::Committing
                | TaskStatus::NeedsReview
                | TaskStatus::Approved
                | TaskStatus::Succeeded
                | TaskStatus::Failed
                | TaskStatus::Abandoned,
            ) => {
                return Err(TaskEngineError::AlreadyTerminal);
            }
            (TaskCommand::Run, status) | (TaskCommand::Cancel, status) => {
                return Err(TaskEngineError::InvalidTransition {
                    from: format!("{status:?}"),
                    operation: match command {
                        TaskCommand::Run => "run",
                        TaskCommand::Cancel => "cancel",
                    },
                });
            }
        }
        self.task(task_id)?.ok_or(TaskEngineError::NotFound("task"))
    }

    fn claim(&mut self, request: ClaimRequest) -> Result<Option<ClaimedTask>, TaskEngineError> {
        if request.worker_id.trim().is_empty() || request.lease_until_ms <= request.now_ms {
            return Err(TaskEngineError::InvalidTransition {
                from: "invalid claim input".to_owned(),
                operation: "claim",
            });
        }
        let Some(task) = self.control_db.next_queued().map_err(map_db)? else {
            return Ok(None);
        };
        let spec = decode_spec(&task)?;

        // All Vault I/O precedes the claim transaction; the queue never selects new input bytes.
        let snapshot_id: SnapshotId =
            serde_json::from_value(json!(task.input_snapshot.as_ref().ok_or_else(|| {
                TaskEngineError::Integrity("queued task has no confirmed snapshot".to_owned())
            })?))?;
        let snapshot = self
            .broker
            .load_snapshot(&snapshot_id)
            .map_err(TaskEngineError::Broker)?;
        let snapshot_record = snapshot_record(&snapshot);
        if snapshot.vault_uuid != self.control_db.bound_vault_uuid().map_err(map_db)?
            || self
                .control_db
                .snapshot(snapshot.id.as_str())
                .map_err(map_db)?
                .as_ref()
                != Some(&snapshot_record)
        {
            return Err(TaskEngineError::Integrity(
                "confirmed snapshot does not match Control DB metadata or Vault identity"
                    .to_owned(),
            ));
        }
        if !self.inputs_unchanged(&task, &snapshot)? {
            self.control_db
                .require_reconfirmation(task.task_id, request.now_ms)
                .map_err(map_db)?;
            return Err(TaskEngineError::NeedsReconfirmation(task.task_id));
        }
        let attempt = self
            .control_db
            .claim(
                task.task_id,
                &request.worker_id,
                snapshot_record,
                request.now_ms,
                request.lease_until_ms,
            )
            .map_err(map_db)?;
        let run_spec = RunSpec {
            task_id: task.task_id,
            attempt_id: attempt.attempt_id,
            fence: attempt.fence,
            instruction: spec
                .instruction
                .clone()
                .or(spec.title.clone())
                .unwrap_or_default(),
            inputs: snapshot
                .files
                .into_iter()
                .map(|file| RunInput {
                    path: file.path,
                    revision: file.revision.to_string(),
                    content: file.content,
                })
                .collect(),
        };
        Ok(Some(ClaimedTask {
            snapshot_id: attempt.snapshot_id,
            run_spec,
        }))
    }

    fn attach_runner(
        &mut self,
        task_id: TaskId,
        attempt_id: AttemptId,
        fence: Fence,
        runner_ref: RunnerRef,
        now_ms: i64,
    ) -> Result<(), TaskEngineError> {
        let task = self
            .task(task_id)?
            .ok_or(TaskEngineError::NotFound("task"))?;
        if runner_ref.adapter != task.spec.adapter
            || runner_ref.opaque_id.trim().is_empty()
            || runner_ref.pid.is_some()
            || runner_ref.start_time_ticks.is_some()
        {
            return Err(TaskEngineError::InvalidRunnerRef);
        }
        let encoded = serde_json::to_value(runner_ref)?;
        self.control_db
            .attach_runner(task_id, attempt_id, fence, &encoded, now_ms)
            .map_err(map_db)
    }

    fn heartbeat(
        &mut self,
        task_id: TaskId,
        attempt_id: AttemptId,
        fence: Fence,
        lease_until_ms: i64,
        now_ms: i64,
    ) -> Result<(), TaskEngineError> {
        if lease_until_ms <= now_ms {
            return Err(TaskEngineError::InvalidTransition {
                from: "expired lease".to_owned(),
                operation: "heartbeat",
            });
        }
        self.control_db
            .heartbeat(task_id, attempt_id, fence, lease_until_ms, now_ms)
            .map_err(map_db)
    }

    fn report(
        &mut self,
        event_id: &str,
        report: RunnerReport,
        now_ms: i64,
    ) -> Result<ReportResult, TaskEngineError> {
        self.verify_vault_binding()?;
        let event_id = AuditEventId::parse(event_id.to_owned()).map_err(map_audit)?;
        let reservation = reservation_for(&event_id, &report)?;
        let prior = self
            .control_db
            .attempt(report.task_id, report.attempt_id)
            .map_err(map_db)?
            .ok_or(TaskEngineError::NotFound("attempt"))?;
        if prior.fence != report.fence {
            return Err(TaskEngineError::StaleFence);
        }
        let replayed = match prior.phase {
            AttemptPhase::Running => {
                self.control_db
                    .reserve_terminal(
                        report.task_id,
                        report.attempt_id,
                        report.fence,
                        &reservation,
                        now_ms,
                    )
                    .map_err(map_db)?;
                false
            }
            AttemptPhase::Committing | AttemptPhase::Completed => {
                if prior.terminal.as_ref() != Some(&reservation) {
                    return Err(TaskEngineError::IdempotencyMismatch);
                }
                prior.phase == AttemptPhase::Completed
            }
            AttemptPhase::Claimed
            | AttemptPhase::CancelRequested
            | AttemptPhase::Failed
            | AttemptPhase::Cancelled
            | AttemptPhase::Abandoned => return Err(TaskEngineError::AlreadyTerminal),
        };
        let receipt = if replayed {
            let receipt = self
                .control_db
                .artifact_receipt(prior.task_id, prior.attempt_id)
                .map_err(map_db)?
                .ok_or_else(|| {
                    TaskEngineError::Integrity("completed attempt has no receipt".to_owned())
                })?;
            self.deliver_pending_outcomes(Some(prior.attempt_id), now_ms)?;
            serde_json::from_value(receipt)?
        } else {
            self.roll_forward(&prior_identity(&prior), &reservation, now_ms)?
        };
        Ok(ReportResult {
            status: TaskStatus::NeedsReview,
            receipt,
            replayed,
        })
    }

    fn reconcile(&mut self, now_ms: i64) -> Result<ReconcileReport, TaskEngineError> {
        self.verify_vault_binding()?;
        let abandoned_attempts = self.control_db.abandon_inflight(now_ms).map_err(map_db)?;
        // M0 only runs the in-process Stub. A process restart has already isolated its work.
        let cancelling = self.control_db.cancelling_attempts().map_err(map_db)?;
        for attempt in &cancelling {
            self.control_db
                .acknowledge_cancel(attempt.task_id, attempt.attempt_id, attempt.fence, now_ms)
                .map_err(map_db)?;
        }
        let audit_outcomes = self.deliver_pending_outcomes(None, now_ms)?;
        let committing = self.control_db.committing_attempts().map_err(map_db)?;
        let mut committed_attempts = 0;
        for attempt in committing {
            let reservation = attempt.terminal.clone().ok_or_else(|| {
                TaskEngineError::Integrity(format!(
                    "committing attempt {} has no reservation",
                    attempt.attempt_id
                ))
            })?;
            self.roll_forward(&prior_identity(&attempt), &reservation, now_ms)?;
            committed_attempts += 1;
        }
        Ok(ReconcileReport {
            abandoned_attempts,
            cancelled_attempts: cancelling.len(),
            committed_attempts,
            audit_outcomes,
        })
    }

    fn acknowledge_cancel(
        &mut self,
        task_id: TaskId,
        attempt_id: AttemptId,
        fence: Fence,
        now_ms: i64,
    ) -> Result<TaskView, TaskEngineError> {
        self.control_db
            .acknowledge_cancel(task_id, attempt_id, fence, now_ms)
            .map_err(map_db)?;
        self.task(task_id)?.ok_or(TaskEngineError::NotFound("task"))
    }

    fn project_pending(&mut self) -> Result<usize, TaskEngineError> {
        self.verify_vault_binding()?;
        let pending = self.control_db.pending_outbox().map_err(map_db)?;
        self.project_events(pending)
    }

    fn task(&self, task_id: TaskId) -> Result<Option<TaskView>, TaskEngineError> {
        self.control_db
            .task(task_id)
            .map_err(map_db)?
            .map(|record| task_view(&record))
            .transpose()
    }
}

impl<B, L> PersistentTaskEngine<B, L>
where
    B: VaultBroker,
    L: AuditLedger,
{
    /// Delivers only the pending events of one task, in sequence order.
    fn project_task(&mut self, task_id: TaskId) -> Result<usize, TaskEngineError> {
        self.verify_vault_binding()?;
        let pending = self
            .control_db
            .pending_outbox()
            .map_err(map_db)?
            .into_iter()
            .filter(|event| event.task_id == task_id)
            .collect();
        self.project_events(pending)
    }

    /// Delivers events in sequence order. A page-level rejection quarantines that task for
    /// the rest of the pass (its later events stay pending, preserving per-task order) without
    /// blocking other tasks; storage and Broker failures still abort the whole pass.
    fn project_events(&mut self, events: Vec<OutboxRecord>) -> Result<usize, TaskEngineError> {
        let mut delivered = 0;
        let mut failures: Vec<ProjectionFailure> = Vec::new();
        for event in events {
            if failures
                .iter()
                .any(|failure| failure.task_id == event.task_id)
            {
                continue;
            }
            match self.project_event(&event) {
                Ok(()) => delivered += 1,
                Err(error) if is_page_rejection(&error) => failures.push(ProjectionFailure {
                    task_id: event.task_id,
                    sequence: event.sequence,
                    reason: error.to_string(),
                }),
                Err(error) => return Err(error),
            }
        }
        if failures.is_empty() {
            Ok(delivered)
        } else {
            Err(TaskEngineError::ProjectionIncomplete {
                delivered,
                failures,
            })
        }
    }

    fn project_event(&mut self, event: &OutboxRecord) -> Result<(), TaskEngineError> {
        {
            let page = task_page_path(event.task_id)?;
            let vault_uuid = self.broker.vault_uuid();
            let mut succeeded = false;
            // 幂等键的判别位：只在「前提当下仍然成立、拒绝只可能是旧键终态的重放」时推进，
            // 详见 `projection::key`。
            let mut generation: u32 = 0;
            for _ in 0..MAX_PROJECTION_ATTEMPTS {
                let mutation = match self.broker.read(&page) {
                    Ok(current) => {
                        projection::validate_task_page(&current.content, event.task_id)
                            .map_err(TaskEngineError::Projection)?;
                        let content =
                            projection::apply_projection(&current.content, &event.payload_json)
                                .map_err(TaskEngineError::Projection)?;
                        if content == current.content {
                            succeeded = true;
                            break;
                        }
                        let key = projection::key::mutation_key(
                            vault_uuid,
                            event.task_id,
                            event.sequence,
                            &page,
                            Precondition::Match(current.revision),
                            &content,
                            generation,
                        );
                        Mutation::replace(page.clone(), content, current.revision, key)
                    }
                    Err(BrokerError::NotFound(_))
                        if projected_status(&event.payload_json) == Some(TaskStatus::Draft) =>
                    {
                        let task = self
                            .control_db
                            .task(event.task_id)
                            .map_err(map_db)?
                            .ok_or(TaskEngineError::NotFound("projection task"))?;
                        let created = jiff::Timestamp::from_millisecond(task.created_at_ms)
                            .map_err(|error| TaskEngineError::Projection(error.to_string()))?
                            .to_string();
                        let content = projection::render_task_page(
                            event.task_id,
                            &decode_spec(&task)?,
                            &created,
                        )
                        .map_err(TaskEngineError::Projection)?;
                        let content = projection::apply_projection(&content, &event.payload_json)
                            .map_err(TaskEngineError::Projection)?;
                        let key = projection::key::mutation_key(
                            vault_uuid,
                            event.task_id,
                            event.sequence,
                            &page,
                            Precondition::MustNotExist,
                            &content,
                            generation,
                        );
                        Mutation::create(page.clone(), content, key)
                    }
                    Err(error) => return Err(TaskEngineError::Broker(error)),
                };
                let precondition = mutation.precondition;
                let expected_content = mutation.content.clone();
                match self.broker.mutate(mutation) {
                    Ok(receipt) => {
                        // An idempotent receipt proves a historical write, not that an
                        // external editor has left those bytes installed since then.
                        let installed = self.broker.read(&page).map_err(TaskEngineError::Broker)?;
                        if installed.revision == receipt.revision
                            && installed.content == expected_content
                        {
                            succeeded = true;
                            break;
                        }
                    }
                    Err(BrokerError::RevisionConflict { .. }) => {
                        // 拒绝要么是对当前状态的判定，要么是 Broker 为这把键封存的拒绝终态的
                        // 重放。前提当下仍然成立 ⇒ 只可能是重放：推进判别位换一把没被封死的
                        // 键，否则这条事件会永远撞在同一个终态上（T-028 起 CAS 拒绝按键终态）。
                        // 前提确实已变时不推进：新前提本身就会派生出新键。
                        if self.premise_still_holds(&page, precondition)? {
                            generation = generation.saturating_add(1);
                        }
                        continue;
                    }
                    Err(BrokerError::IdempotencyConflict) => {
                        // 同一把键被另一个请求占用：明确拒绝，绝不静默继承它的终态。
                        // 只隔离本任务，别的任务的投影照常交付。
                        return Err(TaskEngineError::Projection(format!(
                            "task {} projection idempotency key is already bound to a different request",
                            event.task_id
                        )));
                    }
                    Err(error) => return Err(TaskEngineError::Broker(error)),
                }
            }
            if !succeeded {
                return Err(TaskEngineError::Projection(format!(
                    "task {} changed during {MAX_PROJECTION_ATTEMPTS} projection attempts",
                    event.task_id
                )));
            }
            self.control_db
                .mark_outbox_delivered(event.sequence, jiff::Timestamp::now().as_millisecond())
                .map_err(map_db)?;
        }
        Ok(())
    }

    /// 本次写入的前提（不存在 / 指定 revision）当下是否仍然成立。
    ///
    /// 只用来解释一次 CAS 拒绝：前提仍成立时，这次拒绝不可能是对当前状态的判定，
    /// 只能是 Broker 重放同一把幂等键上的旧拒绝终态。
    fn premise_still_holds(
        &self,
        page: &VaultPath,
        precondition: Precondition,
    ) -> Result<bool, TaskEngineError> {
        match (self.broker.read(page), precondition) {
            (Ok(current), Precondition::Match(expected)) => Ok(current.revision == expected),
            (Ok(_), Precondition::MustNotExist) => Ok(false),
            (Err(BrokerError::NotFound(_)), Precondition::MustNotExist) => Ok(true),
            (Err(BrokerError::NotFound(_)), Precondition::Match(_)) => Ok(false),
            (Err(error), _) => Err(TaskEngineError::Broker(error)),
        }
    }
}

/// Failures caused by the content of one task's own page: they must not block other tasks.
fn is_page_rejection(error: &TaskEngineError) -> bool {
    matches!(
        error,
        TaskEngineError::Projection(_)
            | TaskEngineError::NotFound("projection task")
            | TaskEngineError::Json(_)
    )
}

impl<B, L> PersistentTaskEngine<B, L>
where
    B: VaultBroker,
    L: AuditLedger,
{
    fn verify_vault_binding(&self) -> Result<(), TaskEngineError> {
        if self.broker.vault_uuid() != self.control_db.bound_vault_uuid().map_err(map_db)? {
            return Err(TaskEngineError::Integrity(
                "Broker and Control DB belong to different Vaults".to_owned(),
            ));
        }
        Ok(())
    }

    fn capture_inputs(
        &self,
        task_id: TaskId,
        spec: &TaskSpec,
    ) -> Result<Snapshot, TaskEngineError> {
        let mut paths = spec.input_paths.clone();
        paths.push(task_page_path(task_id)?);
        paths.sort();
        paths.dedup();
        let mut inputs = Vec::with_capacity(paths.len());
        for path in paths {
            let object = self.broker.read(&path).map_err(TaskEngineError::Broker)?;
            inputs.push(SnapshotInput {
                path,
                expected_revision: object.revision,
            });
        }
        let snapshot = self
            .broker
            .snapshot(&inputs)
            .map_err(TaskEngineError::Broker)?;
        if snapshot.vault_uuid != self.control_db.bound_vault_uuid().map_err(map_db)? {
            return Err(TaskEngineError::Integrity(
                "Broker snapshot belongs to another Vault".to_owned(),
            ));
        }
        Ok(snapshot)
    }

    fn inputs_unchanged(
        &self,
        task: &TaskRecord,
        snapshot: &Snapshot,
    ) -> Result<bool, TaskEngineError> {
        let page = task_page_path(task.task_id)?;
        let comparison = json!({
            "status": "queued", "active_attempt": null, "last_attempt": null, "input_snapshot": null,
        });
        for input in &snapshot.files {
            let current = match self.broker.read(&input.path) {
                Ok(value) => value,
                Err(BrokerError::NotFound(_)) => return Ok(false),
                Err(error) => return Err(TaskEngineError::Broker(error)),
            };
            if input.path == page {
                projection::validate_task_page(&current.content, task.task_id)
                    .map_err(TaskEngineError::Projection)?;
                let prior = projection::apply_projection(&input.content, &comparison)
                    .map_err(TaskEngineError::Projection)?;
                let current = projection::apply_projection(&current.content, &comparison)
                    .map_err(TaskEngineError::Projection)?;
                if current != prior {
                    return Ok(false);
                }
            } else if current.revision != input.revision {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn roll_forward(
        &mut self,
        attempt: &AttemptIdentity,
        reservation: &TerminalReservation,
        now_ms: i64,
    ) -> Result<ArtifactCommitReceipt, TaskEngineError> {
        let plan = decode_plan(attempt, reservation)?;
        let subject = AuditSubject::default()
            .with_task(attempt.task_id)
            .with_attempt(attempt.attempt_id);
        let intent_payload = SanitizedPayload::try_from(json!({
            "fence": attempt.fence.get(),
            "artifact_count": plan.artifacts.len(),
        }))
        .map_err(map_audit)?;
        self.audit_ledger
            .append(
                AuditEvent::new(
                    plan.event_id.clone(),
                    AuditPhase::Intent,
                    "task_engine.report",
                    intent_payload,
                )
                .with_subject(subject),
            )
            .map_err(map_audit)?;

        let commit = ArtifactCommit {
            task_id: attempt.task_id,
            attempt_id: attempt.attempt_id,
            fence: attempt.fence,
            audit_intent: plan.event_id.to_string(),
            idempotency_key: format!("task-engine:{}:{}", attempt.attempt_id, plan.event_id),
            artifacts: plan.artifacts,
        };
        let receipt = self
            .broker
            .commit_attempt_artifacts(commit)
            .map_err(TaskEngineError::CommitFailed)?;

        let encoded_receipt = serde_json::to_value(&receipt)?;
        self.control_db
            .finalize_terminal(
                attempt.task_id,
                attempt.attempt_id,
                attempt.fence,
                TaskStatus::NeedsReview,
                Some(&encoded_receipt),
                now_ms,
            )
            .map_err(map_db)?;
        // Audit may only claim a workflow status after the authoritative T2 commit.
        // T2's outbox survives a crash or unavailable ledger at this exact boundary.
        self.deliver_pending_outcomes(Some(attempt.attempt_id), now_ms)?;
        Ok(receipt)
    }

    fn deliver_pending_outcomes(
        &mut self,
        only_attempt: Option<AttemptId>,
        now_ms: i64,
    ) -> Result<usize, TaskEngineError> {
        let pending = self.control_db.pending_terminal_audits().map_err(map_db)?;
        let mut delivered = 0;
        for outcome in pending {
            if only_attempt.is_some_and(|id| id != outcome.attempt.attempt_id) {
                continue;
            }
            self.deliver_outcome(&outcome)?;
            self.control_db
                .mark_terminal_audit_delivered(outcome.attempt.attempt_id, now_ms)
                .map_err(map_db)?;
            delivered += 1;
        }
        Ok(delivered)
    }

    fn deliver_outcome(&self, outcome: &PendingTerminalAudit) -> Result<(), TaskEngineError> {
        let reservation = outcome.attempt.terminal.as_ref().ok_or_else(|| {
            TaskEngineError::Integrity("pending audit has no report reservation".to_owned())
        })?;
        let plan = decode_plan(&prior_identity(&outcome.attempt), reservation)?;
        let receipt: ArtifactCommitReceipt = serde_json::from_value(outcome.receipt_json.clone())?;
        let payload = SanitizedPayload::try_from(json!({
            "status": outcome.status,
            "manifest_path": receipt.manifest_path,
            "manifest_revision": receipt.manifest_revision,
            "artifact_count": receipt.artifacts.len(),
        }))
        .map_err(map_audit)?;
        self.audit_ledger
            .append(
                AuditEvent::new(
                    plan.event_id,
                    AuditPhase::Outcome,
                    "task_engine.report",
                    payload,
                )
                .with_subject(
                    AuditSubject::default()
                        .with_task(outcome.attempt.task_id)
                        .with_attempt(outcome.attempt.attempt_id),
                ),
            )
            .map_err(map_audit)?;
        Ok(())
    }
}

fn task_page_path(task_id: TaskId) -> Result<VaultPath, TaskEngineError> {
    VaultPath::parse(format!("ops/tasks/{task_id}.md"))
        .map_err(|error| TaskEngineError::Integrity(error.to_string()))
}

fn snapshot_record(snapshot: &Snapshot) -> SnapshotRecord {
    SnapshotRecord {
        snapshot_id: snapshot.id.as_str().to_owned(),
        vault_uuid: snapshot.vault_uuid,
        entries: snapshot
            .files
            .iter()
            .map(|file| SnapshotEntryRecord {
                path: file.path.to_string(),
                revision: file.revision.to_string(),
                effective_label_json: json!({"classification": "restricted"}),
            })
            .collect(),
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct PersistedReport {
    event_id: AuditEventId,
    report: RunnerReport,
}

struct CommitPlan {
    event_id: AuditEventId,
    artifacts: Vec<ArtifactCandidate>,
}

#[derive(Clone, Copy)]
struct AttemptIdentity {
    task_id: TaskId,
    attempt_id: AttemptId,
    fence: Fence,
}

fn prior_identity(attempt: &AttemptRecord) -> AttemptIdentity {
    AttemptIdentity {
        task_id: attempt.task_id,
        attempt_id: attempt.attempt_id,
        fence: attempt.fence,
    }
}

fn reservation_for(
    event_id: &AuditEventId,
    report: &RunnerReport,
) -> Result<TerminalReservation, TaskEngineError> {
    let artifacts: Vec<_> = report
        .artifacts
        .iter()
        .map(|artifact| ArtifactCandidate {
            name: artifact.name.clone(),
            content: artifact.content.clone(),
        })
        .collect();
    Ok(TerminalReservation {
        report_json: serde_json::to_value(PersistedReport {
            event_id: event_id.clone(),
            report: report.clone(),
        })?,
        artifacts_json: serde_json::to_value(artifacts)?,
    })
}

fn decode_plan(
    attempt: &AttemptIdentity,
    reservation: &TerminalReservation,
) -> Result<CommitPlan, TaskEngineError> {
    let persisted: PersistedReport = serde_json::from_value(reservation.report_json.clone())?;
    let artifacts: Vec<ArtifactCandidate> =
        serde_json::from_value(reservation.artifacts_json.clone())?;
    if persisted.report.task_id != attempt.task_id
        || persisted.report.attempt_id != attempt.attempt_id
        || persisted.report.fence != attempt.fence
    {
        return Err(TaskEngineError::Integrity(
            "reserved report identity does not match attempt".to_owned(),
        ));
    }
    let declared: Vec<_> = persisted
        .report
        .artifacts
        .iter()
        .map(|artifact| ArtifactCandidate {
            name: artifact.name.clone(),
            content: artifact.content.clone(),
        })
        .collect();
    if declared != artifacts {
        return Err(TaskEngineError::Integrity(
            "reserved artifacts do not match runner report".to_owned(),
        ));
    }
    Ok(CommitPlan {
        event_id: persisted.event_id,
        artifacts,
    })
}

fn task_view(record: &TaskRecord) -> Result<TaskView, TaskEngineError> {
    Ok(TaskView {
        task_id: record.task_id,
        spec: decode_spec(record)?,
        status: record.status,
        active_attempt: record.active_attempt,
        last_attempt: record.last_attempt,
        input_snapshot: record.input_snapshot.clone(),
    })
}

fn decode_spec(record: &TaskRecord) -> Result<TaskSpec, TaskEngineError> {
    let spec: TaskSpec = serde_json::from_value(record.definition_json.clone())?;
    spec.validate()?;
    Ok(spec)
}

/// 投影载荷里的 task 状态：载荷是 Control DB 写出的 JSON，按公开枚举解析而不比较状态名字符串。
fn projected_status(payload: &serde_json::Value) -> Option<TaskStatus> {
    serde_json::from_value(payload.get("status")?.clone()).ok()
}

fn map_db(error: DbError) -> TaskEngineError {
    match error {
        DbError::NotFound(record) => TaskEngineError::NotFound(record),
        // 终态判断只认 Control DB 的结构化变体，不解释状态名字符串（契约 §6）。
        DbError::AlreadyTerminal { .. } => TaskEngineError::AlreadyTerminal,
        DbError::InvalidTransition { from, operation } => {
            TaskEngineError::InvalidTransition { from, operation }
        }
        DbError::FenceMismatch => TaskEngineError::StaleFence,
        other => TaskEngineError::ControlDb(other),
    }
}

fn map_audit(error: AuditError) -> TaskEngineError {
    match error {
        AuditError::Unavailable => TaskEngineError::AuditUnavailable,
        AuditError::IdempotencyConflict { .. } => TaskEngineError::IdempotencyMismatch,
        other => TaskEngineError::Audit(other),
    }
}
