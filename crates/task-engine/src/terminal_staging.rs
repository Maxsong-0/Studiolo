//! 内联的 v1/v2 预留先经 Broker 持久化，再从数据库中移除正文。
use super::*;
use studiolo_vault_broker::StagedArtifactCommit;

impl<B: VaultBroker + ArtifactStager, L: AuditLedger> PersistentTaskEngine<B, L> {
    pub(super) fn commit_for_report(
        &mut self,
        event: &AuditEventId,
        report: &RunnerReport,
    ) -> Result<ArtifactCommit, TaskEngineError> {
        let provenance = self.bound_provenance(&AttemptIdentity {
            task_id: report.task_id,
            attempt_id: report.attempt_id,
            fence: report.fence,
        })?;
        Ok(ArtifactCommit {
            task_id: report.task_id,
            attempt_id: report.attempt_id,
            fence: report.fence,
            audit_intent: event.to_string(),
            idempotency_key: format!("task-engine:{}:{}", report.attempt_id, event),
            artifacts: report
                .artifacts
                .iter()
                .map(|a| ArtifactCandidate {
                    name: a.name.clone(),
                    content: a.content.clone(),
                })
                .collect(),
            provenance,
        })
    }

    pub(super) fn stage_commit(
        &self,
        commit: &ArtifactCommit,
    ) -> Result<TerminalReservation, TaskEngineError> {
        let pointer = self
            .broker
            .stage_artifact_commit(commit)
            .map_err(TaskEngineError::Broker)?;
        pointer_reservation(commit, &pointer)
    }

    pub(super) fn predicted_reservation(
        &self,
        commit: &ArtifactCommit,
    ) -> Result<TerminalReservation, TaskEngineError> {
        let pointer = self
            .broker
            .staged_reference(commit)
            .map_err(TaskEngineError::Broker)?;
        pointer_reservation(commit, &pointer)
    }

    pub(super) fn discard_reservation(
        &self,
        reservation: &TerminalReservation,
    ) -> Result<(), TaskEngineError> {
        if reservation.artifacts_json.is_array() {
            return Ok(());
        }
        let pointer: StagedArtifactCommit =
            serde_json::from_value(reservation.artifacts_json.clone())?;
        self.broker
            .discard_staged_artifact_commit(&pointer)
            .map_err(TaskEngineError::Broker)
    }

    pub(super) fn staged_plan(
        &self,
        attempt: &AttemptIdentity,
        reservation: &TerminalReservation,
    ) -> Result<CommitPlan, TaskEngineError> {
        if reservation.artifacts_json.is_array() {
            return decode_plan(attempt, reservation);
        }
        let pointer: StagedArtifactCommit =
            serde_json::from_value(reservation.artifacts_json.clone())?;
        let commit = self
            .broker
            .load_staged_artifact_commit(&pointer)
            .map_err(TaskEngineError::Broker)?;
        let mut metadata = serde_json::json!({"format":3,"event_id":commit.audit_intent,"task_id":commit.task_id,"attempt_id":commit.attempt_id,"fence":commit.fence});
        if reservation
            .report_json
            .get("outcome")
            .and_then(serde_json::Value::as_str)
            == Some("stopped")
        {
            metadata["outcome"] = serde_json::json!("stopped");
        }
        if commit.task_id != attempt.task_id
            || commit.attempt_id != attempt.attempt_id
            || commit.fence != attempt.fence
            || reservation.report_json != metadata
        {
            return Err(TaskEngineError::Integrity(
                "staged artifact identity differs".into(),
            ));
        }
        Ok(CommitPlan {
            event_id: AuditEventId::parse(commit.audit_intent).map_err(map_audit)?,
            artifacts: commit.artifacts,
            staged_provenance: Some(commit.provenance),
        })
    }

    /// 来源只取原批准和原账本。报告里没有授权字段，旧 attempt 缺记录时不补完整范围。
    pub(super) fn bound_provenance(
        &mut self,
        attempt: &AttemptIdentity,
    ) -> Result<ArtifactProvenance, TaskEngineError> {
        self.deliver_runtime_audits()?;
        let stored = self
            .control_db
            .artifact_provenance(attempt.task_id, attempt.attempt_id, attempt.fence)
            .map_err(map_db)?
            .ok_or_else(|| {
                TaskEngineError::Integrity("旧 attempt 没有产物批准，不能补成完整范围".into())
            })?;
        let expected = self
            .control_db
            .artifact_approval_event(attempt.task_id, attempt.attempt_id, attempt.fence)
            .map_err(map_db)?
            .ok_or_else(|| TaskEngineError::Integrity("原账本缺少产物批准事实".into()))?;
        let mut records = self
            .audit_ledger
            .query(
                &AuditFilter::default()
                    .with_event_id(expected.event_id().clone())
                    .with_limit(2),
            )
            .map_err(map_audit)?;
        let Some(record) = records.pop() else {
            return Err(TaskEngineError::Integrity(
                "原账本的产物批准不是唯一事实".into(),
            ));
        };
        if !records.is_empty() {
            return Err(TaskEngineError::Integrity(
                "原账本的产物批准不是唯一事实".into(),
            ));
        }
        record.verify_integrity().map_err(map_audit)?;
        if record.event() != &expected {
            return Err(TaskEngineError::Integrity("原账本与原批准不一致".into()));
        }
        Ok(stored)
    }

    /// 返回这一轮安全迁入内容面 staging 的旧预留数量。
    ///
    /// 逐个迁（复审 D3-F5）：只和某一份预留有关的失败——M0 时代的 attempt 没有原批准、预留内容对不上、
    /// Broker 拒绝暂存——只让它原样留在控制库、记一条 `task.legacy_reservation_pending`（同一个 attempt
    /// 只记一次），不挡别的 attempt，也不挡之后的启动恢复和周期对账。控制库或审计本身出错照旧整轮失败。
    pub fn migrate_terminal_bodies(&mut self) -> Result<usize, TaskEngineError> {
        let attempts = self.control_db.legacy_terminal_attempts().map_err(map_db)?;
        let mut migrated = 0;
        for attempt in &attempts {
            match self.migrate_terminal_body(attempt) {
                Ok(()) => migrated += 1,
                Err(error) if reservation_specific(&error) => {
                    self.board_pending_marker(
                        "task.legacy_reservation_pending",
                        &(attempt.task_id, attempt.attempt_id),
                    )?;
                }
                Err(error) => return Err(error),
            }
        }
        Ok(migrated)
    }

    fn migrate_terminal_body(&mut self, attempt: &AttemptRecord) -> Result<(), TaskEngineError> {
        let prior = attempt
            .terminal
            .as_ref()
            .ok_or_else(|| TaskEngineError::Integrity("legacy reservation missing".into()))?;
        let identity = prior_identity(attempt);
        let plan = decode_plan(&identity, prior)?;
        if plan.staged_provenance.is_some() {
            return Err(TaskEngineError::Integrity(
                "旧内联预留不能自带完整范围".into(),
            ));
        }
        let commit = ArtifactCommit {
            task_id: attempt.task_id,
            attempt_id: attempt.attempt_id,
            fence: attempt.fence,
            audit_intent: plan.event_id.to_string(),
            idempotency_key: format!("task-engine:{}:{}", attempt.attempt_id, plan.event_id),
            artifacts: plan.artifacts,
            provenance: self.bound_provenance(&identity)?,
        };
        let pointer = self
            .broker
            .stage_artifact_commit(&commit)
            .map_err(TaskEngineError::Broker)?;
        self.control_db
            .migrate_terminal_pointer(
                attempt.attempt_id,
                prior,
                &pointer_reservation(&commit, &pointer)?,
            )
            .map_err(map_db)
    }
}

/// 只和某一个 attempt 的预留有关的失败：预留或暂存内容对不上、旧格式缺原批准、暂存读不出、Broker 拒绝
/// 提交。这种失败只让那个 attempt 停在待恢复，不能挡住别的 attempt；控制库、审计账本本身出错不算，
/// 那是整轮的问题。Broker 整体出了问题也不算（见 [`broker_wide`]）。
pub(super) fn reservation_specific(error: &TaskEngineError) -> bool {
    match error {
        TaskEngineError::Integrity(_) | TaskEngineError::Json(_) => true,
        TaskEngineError::Broker(error) | TaskEngineError::CommitFailed(error) => {
            !broker_wide(error)
        }
        _ => false,
    }
}

/// Broker 整体出了问题、不只是这一份预留，而且只能在服务外面处理：写者被占、写者状态或配对记录坏了、
/// Vault 身份或根目录变了，或者整个文件系统写满、配额用完、只读。这种失败对每个 attempt 都一样，当成
/// 各自待恢复悄悄跳过的话，常驻循环照转、只剩审计里的记录，没人看得见（复审 A1-F3）；所以整轮报错、
/// 让服务停下来。
///
/// 每一种 Broker 错误都列出来，不用通配：Broker 新加一种错误时这里编译不过，逼着先想清楚它归哪边
/// （复审 R2-F1）。下面这些按单个 attempt 算：
/// - 写者未决、结果不确定、已取消：要经维护收口，维护要服务开着；整轮报错会让服务起不来，反倒收不了口。
///   它们另有出口看得见：内容接口读到「过期」，维护页列出写者状态。
/// - 预算超限：要么是这一份的输入超了，要么是暂存区攒满了；后者要经维护清理，同样要服务开着，
///   别的写入也会报同样的错、看得见。
/// - 没有权限：权限按路径给，一份暂存或一个笔记目录的权限不对只挡涉及它的提交；写者根本身读不了时，
///   别的读写一样失败、看得见。
/// - 其余是这一份涉及的路径、内容、幂等键或记录对不上。
fn broker_wide(error: &BrokerError) -> bool {
    use std::io::ErrorKind;
    match error {
        BrokerError::WriterBusy
        | BrokerError::WriterBindingMismatch
        | BrokerError::CorruptWriterState
        | BrokerError::CorruptIdentity
        | BrokerError::RootChanged => true,
        BrokerError::Io { source, .. } => matches!(
            source.kind(),
            ErrorKind::StorageFull | ErrorKind::QuotaExceeded | ErrorKind::ReadOnlyFilesystem
        ),
        BrokerError::MutationUnresolved
        | BrokerError::MutationCancelled
        | BrokerError::MutationIndeterminate { .. }
        | BrokerError::WriterLimitExceeded
        | BrokerError::CaptureChanged
        | BrokerError::IncompleteCapture
        | BrokerError::ProtectedPath(_)
        | BrokerError::InvalidHistoryCursor
        | BrokerError::HistoryChanged
        | BrokerError::NotFound(_)
        | BrokerError::RevisionConflict { .. }
        | BrokerError::UnsafeNode { .. }
        | BrokerError::PathCollision { .. }
        | BrokerError::IdempotencyConflict
        | BrokerError::InvalidArtifact(_)
        | BrokerError::ArtifactConflict(_)
        | BrokerError::ReadLimitExceeded { .. }
        | BrokerError::CorruptSnapshot
        | BrokerError::UnstableRead(_)
        | BrokerError::Json { .. } => false,
    }
}

fn pointer_reservation(
    commit: &ArtifactCommit,
    pointer: &StagedArtifactCommit,
) -> Result<TerminalReservation, TaskEngineError> {
    Ok(TerminalReservation {
        report_json: serde_json::json!({"format":3,"event_id":commit.audit_intent,"task_id":commit.task_id,"attempt_id":commit.attempt_id,"fence":commit.fence}),
        artifacts_json: serde_json::to_value(pointer)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn io(kind: std::io::ErrorKind) -> BrokerError {
        BrokerError::Io {
            operation: "fixture",
            source: std::io::Error::from(kind),
        }
    }

    /// 只挡这一份预留的失败放过去；Broker 整体出问题、控制库或审计出错都让整轮报错（复审 A1-F3）。
    /// 没有权限、写者未决这一类、预算超限按单个 attempt 算（复审 R2-F1）。
    #[test]
    fn only_reservation_level_failures_are_isolated() {
        use std::io::ErrorKind;
        let isolated = [
            TaskEngineError::Integrity("bad".into()),
            TaskEngineError::Json(serde_json::from_str::<u8>("x").unwrap_err()),
            TaskEngineError::Broker(BrokerError::InvalidArtifact("expired".into())),
            TaskEngineError::CommitFailed(BrokerError::IdempotencyConflict),
            TaskEngineError::CommitFailed(io(ErrorKind::NotFound)),
            TaskEngineError::Broker(io(ErrorKind::Other)),
            TaskEngineError::CommitFailed(io(ErrorKind::PermissionDenied)),
            TaskEngineError::Broker(io(ErrorKind::PermissionDenied)),
            TaskEngineError::CommitFailed(BrokerError::MutationUnresolved),
            TaskEngineError::CommitFailed(BrokerError::MutationIndeterminate {
                evidence_id: "fixture".into(),
            }),
            TaskEngineError::Broker(BrokerError::MutationCancelled),
            TaskEngineError::Broker(BrokerError::WriterLimitExceeded),
        ];
        for error in &isolated {
            assert!(reservation_specific(error), "{error:?}");
        }
        let whole_round = [
            TaskEngineError::Broker(BrokerError::WriterBusy),
            TaskEngineError::Broker(BrokerError::WriterBindingMismatch),
            TaskEngineError::CommitFailed(BrokerError::CorruptWriterState),
            TaskEngineError::Broker(BrokerError::CorruptIdentity),
            TaskEngineError::CommitFailed(BrokerError::RootChanged),
            TaskEngineError::CommitFailed(io(ErrorKind::StorageFull)),
            TaskEngineError::Broker(io(ErrorKind::QuotaExceeded)),
            TaskEngineError::Broker(io(ErrorKind::ReadOnlyFilesystem)),
            TaskEngineError::AuditUnavailable,
            TaskEngineError::NotFound("task"),
        ];
        for error in &whole_round {
            assert!(!reservation_specific(error), "{error:?}");
        }
    }
}
