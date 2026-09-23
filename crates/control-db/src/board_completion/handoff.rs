//! 结构化交接：按 task/attempt 读取已接受的交接；租约有效时经 fence 校验保存，并把任务放回队列。
use super::*;

impl ControlDb {
    /// 读取与 task/attempt 匹配的已接受交接；据此可在释放后精确重放。
    pub fn board_handoff_for_attempt(
        &self,
        task: TaskId,
        attempt: AttemptId,
    ) -> Result<Option<Value>, DbError> {
        let raw:Option<String>=self.connection.query_row("SELECT h.handoff_json FROM board_handoffs h JOIN attempts a USING(attempt_id) WHERE a.task_id=?1 AND a.attempt_id=?2",params![task.to_string(),attempt.to_string()],|r|r.get(0)).optional()?;
        raw.map(|raw| serde_json::from_str(&raw).map_err(DbError::from))
            .transpose()
    }

    /// 仅在当前租约有效时保存已校验的结构化交接；同 fence、同内容的重放直接成功。
    pub fn set_board_handoff(
        &mut self,
        task_id: TaskId,
        attempt_id: AttemptId,
        fence: Fence,
        handoff: &Value,
        now_ms: i64,
    ) -> Result<(), DbError> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let stored_fence: Option<i64> = tx
            .query_row(
                "SELECT fence FROM attempts WHERE task_id=?1 AND attempt_id=?2",
                params![task_id.to_string(), attempt_id.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        if stored_fence != Some(fence_i64(fence)?) {
            return Err(DbError::FenceMismatch);
        }
        let encoded = serde_json::to_string(handoff)?;
        let prior: Option<(i64, String)> = tx
            .query_row(
                "SELECT fence,handoff_json FROM board_handoffs WHERE attempt_id=?1",
                [attempt_id.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((stored, prior)) = prior {
            if stored != fence_i64(fence)? {
                return Err(DbError::FenceMismatch);
            }
            if prior != encoded {
                return Err(DbError::IdempotencyMismatch);
            }
            return Ok(());
        }
        lifecycle::require_live(&tx, task_id, attempt_id, fence, now_ms)?;
        let changed = tx.execute(
            "UPDATE board_tasks SET handoff_json=?2 WHERE task_id=?1",
            params![task_id.to_string(), encoded],
        )?;
        if changed != 1 {
            return Err(DbError::NotFound("board task"));
        }
        tx.execute(
            "INSERT INTO board_handoffs(attempt_id,fence,handoff_json) VALUES(?1,?2,?3)",
            params![attempt_id.to_string(), fence_i64(fence)?, encoded],
        )?;
        tx.execute(
            "UPDATE attempts SET phase='abandoned',updated_at_ms=?2 WHERE attempt_id=?1",
            params![attempt_id.to_string(), now_ms],
        )?;
        tx.execute(
            "UPDATE attempt_progress SET terminal_reason='handed_off' WHERE attempt_id=?1",
            [attempt_id.to_string()],
        )?;
        tx.execute("UPDATE tasks SET status='queued',last_attempt=active_attempt,active_attempt=NULL,updated_at_ms=?2 WHERE task_id=?1",params![task_id.to_string(),now_ms])?;
        runtime_audit::enqueue(
            &tx,
            &format!("handoff:{attempt_id}:{now_ms}"),
            "handoff.created",
            serde_json::json!({"task_id":task_id,"attempt_id":attempt_id,"fence":fence.get()}),
        )?;
        enqueue_projection(&tx, task_id, &format!("handoff:{attempt_id}:{now_ms}"))?;
        tx.commit()?;
        Ok(())
    }
}
