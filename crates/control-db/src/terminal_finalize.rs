//! 终态 T2：落定终态，并在同一事务登记待投递的审计结果。
use crate::{
    AttemptPhase, ControlDb, DbError, TaskStatus, TerminalReservation,
    projection_outbox::enqueue_projection, runtime_audit, stopped_candidates, types::fence_i64,
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

    /// 预留之后原批准就过期了、产物一个字节都没写出去：把这次提交落成终态。
    ///
    /// 只许在 Broker 证实「过期且没开始写」之后调用；这里再核一遍这个 attempt 确实停在「提交中」、
    /// 没有任何产物回执。普通预留落成失败；停止态候选的取消结果在预留时已经冻结，落成取消。
    /// 这是黑板 runner 候选唯一一条不经主人审阅的终态：没有候选可审，也没有任何内容被接受。
    /// 审计事件和状态变化在同一个事务里登记；同一个 attempt 重放返回同一个结果。
    pub fn expire_committing(
        &mut self,
        task_id: TaskId,
        attempt_id: AttemptId,
        fence: Fence,
        now_ms: i64,
    ) -> Result<TaskStatus, DbError> {
        const OPERATION: &str = "expire_committing";
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let state: Option<(String, i64, Option<String>, String)> = tx
            .query_row(
                "SELECT a.phase, a.fence, a.terminal_json, t.status
                 FROM attempts a JOIN tasks t ON t.task_id = a.task_id
                 WHERE a.task_id = ?1 AND a.attempt_id = ?2",
                params![task_id.to_string(), attempt_id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?;
        let (phase, stored_fence, terminal, task_status) =
            state.ok_or(DbError::NotFound("attempt"))?;
        if stored_fence != fence_i64(fence)? {
            return Err(DbError::FenceMismatch);
        }
        let Some(terminal) = terminal else {
            return Err(DbError::InvalidTransition {
                from: phase,
                operation: OPERATION,
            });
        };
        let reservation: TerminalReservation = serde_json::from_str(&terminal)?;
        let stopped = reservation
            .report_json
            .get("outcome")
            .and_then(Value::as_str)
            == Some("stopped");
        let (status, terminal_phase) = if stopped {
            (TaskStatus::Cancelled, AttemptPhase::Cancelled)
        } else {
            (TaskStatus::Failed, AttemptPhase::Failed)
        };
        let marker = format!("commit-expired:{attempt_id}");
        if phase == terminal_phase.as_str() && task_status == status.as_str() {
            // 重放：只有当初就是经这条出口落定的才算数，别的途径失败的不能借它改口。
            let recorded: i64 = tx.query_row(
                "SELECT COUNT(*) FROM runtime_audit_outbox WHERE event_id=?1",
                [&marker],
                |row| row.get(0),
            )?;
            if recorded == 1 {
                tx.commit()?;
                return Ok(status);
            }
        }
        let committed: i64 = tx.query_row(
            "SELECT COUNT(*) FROM artifact_commits WHERE attempt_id=?1",
            [attempt_id.to_string()],
            |row| row.get(0),
        )?;
        if phase != AttemptPhase::Committing.as_str() || committed != 0 {
            return Err(DbError::InvalidTransition {
                from: phase,
                operation: OPERATION,
            });
        }
        runtime_audit::enqueue(
            &tx,
            &marker,
            "task.commit_expired",
            serde_json::json!({
                "task_id": task_id,
                "attempt_id": attempt_id,
                "fence": fence.get(),
                "status": status.as_str(),
            }),
        )?;
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
        Ok(status)
    }

    /// 只读：这个 attempt 是不是已经经 `expire_committing` 落定过；是就给回当时落成的状态。
    ///
    /// 主人重试丢弃时用它跳过证实（暂存可能已经丢了），只把剩下的审计投递和丢暂存做完。
    /// 落定标记在、attempt 的阶段却对不上，按损坏拒绝。
    pub fn expired_commit(
        &self,
        task_id: TaskId,
        attempt_id: AttemptId,
    ) -> Result<Option<TaskStatus>, DbError> {
        const OPERATION: &str = "expired_commit";
        let payload: Option<String> = self
            .connection
            .query_row(
                "SELECT payload_json FROM runtime_audit_outbox WHERE event_id=?1",
                [format!("commit-expired:{attempt_id}")],
                |row| row.get(0),
            )
            .optional()?;
        let Some(payload) = payload else {
            return Ok(None);
        };
        let payload: Value = serde_json::from_str(&payload)?;
        let status =
            TaskStatus::parse(payload.get("status").and_then(Value::as_str).unwrap_or(""))?;
        let phase: Option<String> = self
            .connection
            .query_row(
                "SELECT phase FROM attempts WHERE task_id = ?1 AND attempt_id = ?2",
                params![task_id.to_string(), attempt_id.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        let expected = if status == TaskStatus::Cancelled {
            AttemptPhase::Cancelled
        } else {
            AttemptPhase::Failed
        };
        if phase.as_deref() != Some(expected.as_str()) {
            return Err(DbError::InvalidTransition {
                from: phase.unwrap_or_default(),
                operation: OPERATION,
            });
        }
        Ok(Some(status))
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
