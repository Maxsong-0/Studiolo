//! 结构化外发交接，以及已核验终态回执的接受。
use super::*;
impl<B: VaultBroker + ArtifactStager, L: AuditLedger> PersistentTaskEngine<B, L> {
    /// 保存结构化交接；它不能悄悄改动验收标准或冻结的规则版本。
    pub(super) fn put_handoff_inner(
        &mut self,
        task: TaskId,
        attempt: AttemptId,
        fence: Fence,
        handoff: Handoff,
        now: i64,
    ) -> Result<(), TaskEngineError> {
        bounded(&handoff)?;
        let encoded = serde_json::to_value(&handoff)?;
        if self
            .control_db
            .board_handoff_for_attempt(task, attempt)
            .map_err(map_db)?
            .is_some()
        {
            return self
                .control_db
                .set_board_handoff(task, attempt, fence, &encoded, now)
                .map_err(map_db);
        }
        let board = self
            .control_db
            .board_task(task)
            .map_err(map_db)?
            .ok_or(TaskEngineError::NotFound("board task"))?;
        if handoff.goal.trim().is_empty()
            || handoff.goal.len() > 2048
            || !strings(&handoff.forbidden, true)
            || !strings(&handoff.decided, false)
            || !strings(&handoff.undecided, false)
            || handoff.constraints.is_empty()
            || handoff.constraints.len() > 64
            || handoff.constraints.iter().any(|rule| {
                rule.rule_id.trim().is_empty()
                    || rule.rule_id.len() > 256
                    || rule.version_hash != board.definition.policy_hash
            })
            || handoff.required_files.len() > 256
            || handoff
                .required_files
                .iter()
                .any(|file| file.revision.is_empty() || file.revision.len() > 256)
            || handoff.acceptance_criteria != board.definition.acceptance_criteria
        {
            return Err(reject("invalid or unpinned handoff"));
        }
        for file in &handoff.required_files {
            let object = self
                .broker
                .read(&file.path)
                .map_err(TaskEngineError::Broker)?;
            if object.revision.to_string() != file.revision {
                return Err(reject(
                    "handoff file revision does not match canonical content",
                ));
            }
        }
        self.control_db
            .set_board_handoff(task, attempt, fence, &serde_json::to_value(handoff)?, now)
            .map_err(map_db)
    }

    /// 接受完整的逐条验收回执、收尾决定与无冲突产物。
    pub(super) fn return_board_receipt_inner(
        &mut self,
        task: TaskId,
        attempt: AttemptId,
        fence: Fence,
        receipt: BoardReceipt,
        now: i64,
    ) -> Result<BoardView, TaskEngineError> {
        bounded(&receipt)?;
        let current = self.task(task)?.ok_or(TaskEngineError::NotFound("task"))?;
        if current.status == TaskStatus::NeedsReview
            || (current.spec.adapter != "consumer"
                && matches!(
                    receipt.status,
                    ReceiptStatus::Completed | ReceiptStatus::Stopped
                ))
        {
            return Err(reject("runner candidates require an explicit owner review"));
        }
        let board = self
            .control_db
            .board_task(task)
            .map_err(map_db)?
            .ok_or(TaskEngineError::NotFound("board task"))?;
        validate_receipt(&receipt, &board.definition.acceptance_criteria)?;
        let status = match receipt.status {
            ReceiptStatus::Completed => TaskStatus::Succeeded,
            ReceiptStatus::Failed => TaskStatus::Failed,
            ReceiptStatus::Stopped => TaskStatus::Cancelled,
            ReceiptStatus::Abandoned => TaskStatus::Abandoned,
        };
        if board.receipt.is_some() {
            self.control_db
                .complete_board_task(
                    task,
                    attempt,
                    fence,
                    &serde_json::to_value(&receipt)?,
                    status,
                    now,
                )
                .map_err(map_db)?;
            return self
                .board_detail(task, &board.definition.policy_hash)?
                .ok_or(TaskEngineError::NotFound("board task"));
        }
        for pointer in receipt.artifacts.iter().chain(
            receipt
                .acceptance
                .iter()
                .flat_map(|assessment| assessment.evidence.iter()),
        ) {
            self.verify_artifact_pointer(pointer)?;
        }
        for pointer in &receipt.unlanded {
            let object = self
                .broker
                .read(&pointer.path)
                .map_err(TaskEngineError::Broker)?;
            use crate::board_types::sha256_hex;
            if sha256_hex(&object.content) != pointer.sha256.to_ascii_lowercase() {
                return Err(reject("unlanded patch pointer mismatch"));
            }
        }

        self.control_db
            .complete_board_task(
                task,
                attempt,
                fence,
                &serde_json::to_value(receipt)?,
                status,
                now,
            )
            .map_err(map_db)?;
        self.board_detail(task, &board.definition.policy_hash)?
            .ok_or(TaskEngineError::NotFound("board task"))
    }
}

pub(super) fn validate_receipt(
    receipt: &BoardReceipt,
    criteria: &[String],
) -> Result<(), TaskEngineError> {
    if !receipt.conflicts.is_empty() {
        return Err(reject("receipt has unresolved conflicts"));
    }
    if receipt.acceptance.len() != criteria.len()
        || receipt
            .acceptance
            .iter()
            .zip(criteria)
            .any(|(got, want)| &got.criterion != want)
    {
        return Err(reject("receipt must assess every criterion in order"));
    }
    if matches!(receipt.status, ReceiptStatus::Completed)
        && (receipt
            .acceptance
            .iter()
            .any(|a| a.result != AssessmentResult::Pass)
            || !receipt.unlanded.is_empty()
            || !receipt.remaining.is_empty())
    {
        return Err(reject("completed receipt has incomplete work"));
    }
    if receipt.artifacts.len() > 256
        || receipt.artifacts.iter().any(|p| !valid_pointer(p))
        || receipt.unlanded.len() > 256
        || receipt.unlanded.iter().any(|p| !valid_hash(&p.sha256))
        || receipt
            .acceptance
            .iter()
            .any(|a| a.evidence.len() > 64 || a.evidence.iter().any(|p| !valid_pointer(p)))
        || !strings(&receipt.remaining, false)
        || !strings(&receipt.learned, false)
        || !strings(&receipt.closeout.learned, false)
        || receipt.closeout.destinations.len() > 64
    {
        return Err(reject("invalid bounded receipt"));
    }
    Ok(())
}
