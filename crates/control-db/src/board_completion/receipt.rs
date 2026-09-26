//! 终态回执：runner 回执与主人复核共用一条仲裁路径，产物目标按「首个有效完成者胜出」。
use crate::{
    BoardDefinition, ControlDb, DbError, TaskStatus, board, lifecycle,
    projection_outbox::enqueue_projection, runtime_audit, types::fence_i64,
};
use rusqlite::{OptionalExtension as _, TransactionBehavior, params};
use serde_json::Value;
use studiolo_core::{AttemptId, Fence, TaskId};

impl ControlDb {
    /// 原子地接受回执，并按「首个有效完成者胜出」仲裁产物目标。
    pub fn complete_board_task(
        &mut self,
        task_id: TaskId,
        attempt_id: AttemptId,
        fence: Fence,
        receipt: &Value,
        status: TaskStatus,
        now_ms: i64,
    ) -> Result<(), DbError> {
        self.complete_board_task_kind(task_id, attempt_id, fence, receipt, status, now_ms, false)
    }

    /// 仅限受信任的主人对已处于 NeedsReview 的候选作出终态决定。
    pub fn review_board_candidates(
        &mut self,
        task_id: TaskId,
        attempt_id: AttemptId,
        fence: Fence,
        receipt: &Value,
        status: TaskStatus,
        now_ms: i64,
    ) -> Result<(), DbError> {
        self.complete_board_task_kind(task_id, attempt_id, fence, receipt, status, now_ms, true)
    }

    #[allow(clippy::too_many_arguments)]
    fn complete_board_task_kind(
        &mut self,
        task_id: TaskId,
        attempt_id: AttemptId,
        fence: Fence,
        receipt: &Value,
        status: TaskStatus,
        now_ms: i64,
        owner_review: bool,
    ) -> Result<(), DbError> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (phase,stored_fence,stored_receipt): (String,i64,Option<String>) = tx.query_row(
            "SELECT a.phase,a.fence,b.receipt_json FROM attempts a JOIN board_tasks b USING(task_id) WHERE a.task_id=?1 AND a.attempt_id=?2",
            params![task_id.to_string(),attempt_id.to_string()], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?.ok_or(DbError::NotFound("board attempt"))?;
        if stored_fence != fence_i64(fence)? {
            return Err(DbError::FenceMismatch);
        }
        let authoritative: Option<String> = tx.query_row(
            "SELECT COALESCE(active_attempt,last_attempt) FROM tasks WHERE task_id=?1",
            [task_id.to_string()],
            |r| r.get(0),
        )?;
        if authoritative.as_deref() != Some(attempt_id.to_string().as_str()) {
            return Err(DbError::FenceMismatch);
        }
        let encoded = serde_json::to_string(receipt)?;
        if stored_receipt.as_deref() == Some(encoded.as_str()) {
            return Ok(());
        }
        if stored_receipt.is_some() {
            return Err(DbError::IdempotencyMismatch);
        }
        let adapter: String = tx.query_row(
            "SELECT json_extract(definition_json,'$.adapter') FROM tasks WHERE task_id=?1",
            [task_id.to_string()],
            |r| r.get(0),
        )?;
        if (owner_review && adapter == "consumer")
            || (!owner_review
                && adapter != "consumer"
                && matches!(status, TaskStatus::Succeeded | TaskStatus::Cancelled))
        {
            return Err(DbError::BoardRejected(
                "runner success requires owner review of committed candidates",
            ));
        }
        let terminal_phase = match status {
            TaskStatus::Succeeded => "completed",
            TaskStatus::Failed => "failed",
            TaskStatus::Cancelled => "cancelled",
            TaskStatus::Abandoned => "abandoned",
            _ => return Err(DbError::BoardRejected("invalid receipt status")),
        };
        if owner_review {
            let task_status: String = tx.query_row(
                "SELECT status FROM tasks WHERE task_id=?1",
                [task_id.to_string()],
                |r| r.get(0),
            )?;
            let has_candidates: i64 = tx.query_row(
                "SELECT COUNT(*) FROM artifact_commits WHERE attempt_id=?1",
                [attempt_id.to_string()],
                |r| r.get(0),
            )?;
            if phase != "completed" || task_status != "needs_review" || has_candidates != 1 {
                return Err(DbError::BoardRejected(
                    "owner review requires committed candidates awaiting review",
                ));
            }
        } else if phase == "cancel_requested" {
            let requested: Option<i64> = tx.query_row(
                "SELECT cancel_requested_ms FROM attempt_progress WHERE attempt_id=?1",
                [attempt_id.to_string()],
                |r| r.get(0),
            )?;
            if status != TaskStatus::Cancelled
                || requested.is_none_or(|start| now_ms >= start.saturating_add(180_000))
            {
                return Err(DbError::BoardRejected(
                    "cancel window only accepts stopped receipt",
                ));
            }
        } else {
            lifecycle::require_live(&tx, task_id, attempt_id, fence, now_ms)?;
        }
        board::require_children_terminal(&tx, task_id)?;
        if !receipt
            .get("conflicts")
            .and_then(Value::as_array)
            .is_some_and(Vec::is_empty)
            || receipt.get("closeout").is_none_or(Value::is_null)
        {
            return Err(DbError::BoardRejected("conflicts or missing closeout"));
        }
        let (raw,project): (String,String) = tx.query_row("SELECT b.definition_json,json_extract(t.definition_json,'$.project') FROM board_tasks b JOIN tasks t USING(task_id) WHERE task_id=?1",[task_id.to_string()],|r|Ok((r.get(0)?,r.get(1)?)))?;
        let definition: BoardDefinition = serde_json::from_str(&raw)?;
        if status == TaskStatus::Succeeded
            && let Some(target) = definition.artifact_target
        {
            let winner: Option<String> = tx
                .query_row(
                    "SELECT task_id FROM board_artifact_winners WHERE project=?1 AND target=?2",
                    params![project, target],
                    |r| r.get(0),
                )
                .optional()?;
            if winner
                .as_deref()
                .is_some_and(|id| id != task_id.to_string())
            {
                return Err(DbError::BoardRejected(
                    "artifact target already has a verified winner",
                ));
            }
            tx.execute("INSERT OR IGNORE INTO board_artifact_winners(project,target,task_id) VALUES(?1,?2,?3)",params![project,target,task_id.to_string()])?;
        }
        tx.execute(
            "UPDATE board_tasks SET receipt_json=?2 WHERE task_id=?1",
            params![task_id.to_string(), encoded],
        )?;
        tx.execute(
            "UPDATE attempts SET phase=?2,updated_at_ms=?3 WHERE attempt_id=?1",
            params![attempt_id.to_string(), terminal_phase, now_ms],
        )?;
        tx.execute("UPDATE tasks SET status=?2,active_attempt=NULL,last_attempt=?3,updated_at_ms=?4 WHERE task_id=?1",params![task_id.to_string(),status.as_str(),attempt_id.to_string(),now_ms])?;
        runtime_audit::enqueue(
            &tx,
            &format!("board-terminal:{attempt_id}"),
            if owner_review {
                "task.candidates_reviewed"
            } else {
                "handoff.returned"
            },
            serde_json::json!({"task_id":task_id,"attempt_id":attempt_id,"fence":fence.get(),"status":status}),
        )?;
        enqueue_projection(&tx, task_id, &format!("board-terminal:{attempt_id}"))?;
        tx.commit()?;
        Ok(())
    }
}
