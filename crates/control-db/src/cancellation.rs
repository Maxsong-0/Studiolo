//! 取消：撤销活动能力，并在 runner 隔离后完成取消。
use super::*;

impl ControlDb {
    /// 立即撤销活动能力：非黑板的 draft/queued 直接取消，running 转入 `cancel_requested`。
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

    /// runner 隔离后，完成先前请求的取消。
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
}
