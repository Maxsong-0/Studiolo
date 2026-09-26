//! 终态 T2：落定终态，并在同一事务登记待投递的审计结果。
use crate::{
    AttemptPhase, ControlDb, DbError, TaskStatus, projection_outbox::enqueue_projection,
    runtime_audit, stopped_candidates, types::fence_i64,
};
use rusqlite::{OptionalExtension as _, TransactionBehavior, params};
use serde_json::Value;
use studiolo_core::{AttemptId, Fence, TaskId};

impl ControlDb {
    /// 在审计意图与 Broker 效果之后执行 T2；带 Broker 回执时同事务登记待投递的审计结果。
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
    pub(super) fn finalize_terminal_kind(
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
}
