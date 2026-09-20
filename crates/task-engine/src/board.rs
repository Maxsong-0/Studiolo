//! Machine board API. Authorization stays in the authenticated composition root.
mod candidates;
mod creation;
mod handoff;
mod stopped;
use super::*;
use crate::board_types::{bounded, reject, strings, valid_hash, valid_pointer};
use studiolo_control_db::BoardDefinition;

impl<B: VaultBroker + ArtifactStager, L: AuditLedger> PersistentTaskEngine<B, L> {
    /// Creates an audited board task with required constraints and server-pinned policy.
    pub fn create_board_task(
        &mut self,
        request_id: &str,
        spec: BoardTaskSpec,
        hash: &str,
        now: i64,
    ) -> Result<TaskId, TaskEngineError> {
        let request = (request_id, &spec, hash);
        self.audit_board("task.create", &request, AuditPhase::Intent, None)?;
        let result = self.create_board_task_inner(request_id, spec.clone(), hash, None, now);
        self.audit_board(
            "task.create",
            &request,
            AuditPhase::Outcome,
            Some(result.is_ok()),
        )?;
        result
    }
    /// Freezes the authenticated creator, including when the item is later terminal.
    pub fn create_board_task_by_instance(
        &mut self,
        request_id: &str,
        spec: BoardTaskSpec,
        hash: &str,
        instance: &str,
        now: i64,
    ) -> Result<TaskId, TaskEngineError> {
        if instance.trim().is_empty() || instance.len() > 256 {
            return Err(reject("invalid creator instance"));
        }
        let request = (request_id, &spec, hash, instance);
        self.audit_board("task.create", &request, AuditPhase::Intent, None)?;
        let result =
            self.create_board_task_inner(request_id, spec.clone(), hash, Some(instance), now);
        self.audit_board(
            "task.create",
            &request,
            AuditPhase::Outcome,
            Some(result.is_ok()),
        )?;
        result
    }
    /// Audited claim of one explicit item; expiry is checked in the claim transaction.
    pub fn claim_board_task(
        &mut self,
        task: TaskId,
        instance: &str,
        lease: i64,
        now: i64,
    ) -> Result<BoardClaim, TaskEngineError> {
        let request = (task, instance, lease, now);
        self.audit_board("task.claim", &request, AuditPhase::Intent, None)?;
        let result = self.claim_board_task_inner(task, instance, lease, now);
        self.audit_board(
            "task.claim",
            &request,
            AuditPhase::Outcome,
            Some(result.is_ok()),
        )?;
        result
    }
    /// Audited structured handoff under the current fence.
    pub fn put_handoff(
        &mut self,
        task: TaskId,
        attempt: AttemptId,
        fence: Fence,
        handoff: Handoff,
        now: i64,
    ) -> Result<(), TaskEngineError> {
        let request = (task, attempt, fence, &handoff, now);
        self.audit_board("handoff.created", &request, AuditPhase::Intent, None)?;
        let result = self.put_handoff_inner(task, attempt, fence, handoff.clone(), now);
        self.audit_board(
            "handoff.created",
            &request,
            AuditPhase::Outcome,
            Some(result.is_ok()),
        )?;
        result
    }
    /// Audited terminal receipt; all content pointers are verified before acceptance.
    pub fn return_board_receipt(
        &mut self,
        task: TaskId,
        attempt: AttemptId,
        fence: Fence,
        receipt: BoardReceipt,
        now: i64,
    ) -> Result<BoardView, TaskEngineError> {
        let request = (task, attempt, fence, &receipt, now);
        self.audit_board("handoff.returned", &request, AuditPhase::Intent, None)?;
        let result = self.return_board_receipt_inner(task, attempt, fence, receipt.clone(), now);
        self.audit_board(
            "handoff.returned",
            &request,
            AuditPhase::Outcome,
            Some(result.is_ok()),
        )?;
        result
    }

    /// Checks instance ownership and a current unexpired claim for every protected write.
    pub fn owns_board_claim(
        &self,
        task: TaskId,
        attempt: AttemptId,
        fence: Fence,
        instance: &str,
        now: i64,
    ) -> Result<bool, TaskEngineError> {
        Ok(self.control_db.board_task(task).map_err(map_db)?.is_some()
            && self
                .control_db
                .attempt(task, attempt)
                .map_err(map_db)?
                .is_some_and(|a| a.worker_id == instance)
            && self
                .control_db
                .is_attempt_active_at(task, attempt, fence, now)
                .map_err(map_db)?)
    }

    /// Reads one item with current-policy comparison and live/last lease metadata.
    pub fn board_detail(
        &self,
        task_id: TaskId,
        current_policy_hash: &str,
    ) -> Result<Option<BoardView>, TaskEngineError> {
        let Some(board) = self.control_db.board_task(task_id).map_err(map_db)? else {
            return Ok(None);
        };
        let task = self
            .task(task_id)?
            .ok_or(TaskEngineError::NotFound("task"))?;
        let active = task
            .active_attempt
            .or(task.last_attempt)
            .map(|id| self.control_db.attempt(task_id, id).map_err(map_db))
            .transpose()?
            .flatten();
        let progress = active
            .as_ref()
            .map(|a| {
                self.control_db
                    .attempt_progress(a.attempt_id)
                    .map_err(map_db)
            })
            .transpose()?
            .flatten();
        let candidate_stages = self
            .control_db
            .runner_candidate_stages(task_id)
            .map_err(map_db)?;
        let candidate_receipt = active
            .as_ref()
            .map(|a| {
                self.control_db
                    .artifact_receipt(task_id, a.attempt_id)
                    .map_err(map_db)
            })
            .transpose()?
            .flatten()
            .map(serde_json::from_value)
            .transpose()?;
        Ok(Some(BoardView {
            uses_old_policy: board.definition.policy_hash != current_policy_hash,
            holder: active.as_ref().map(|a| a.worker_id.clone()),
            progress,
            attempt: active,
            candidate_stages,
            candidate_receipt,
            task,
            board,
        }))
    }

    /// Lists only work items in the requested project, preserving authority metadata.
    pub fn board_list(
        &self,
        project: &str,
        current_policy_hash: &str,
    ) -> Result<Vec<BoardView>, TaskEngineError> {
        let mut views = Vec::new();
        for board in self.control_db.board_tasks().map_err(map_db)? {
            let view = self
                .board_detail(board.task_id, current_policy_hash)?
                .ok_or(TaskEngineError::NotFound("board task"))?;
            if view.task.spec.project == project {
                views.push(view);
            }
        }
        Ok(views)
    }
}
