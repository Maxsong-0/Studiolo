//! 黑板回执的 fence 校验、终态仲裁与编号预留。
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

    /// 在一个写事务内取 max(受信任的文件名扫描结果, 已持久预留的最大号)+1。
    /// 只有受信任的 TaskEngine 调用方可以提供 `file_max`，外部客户端永远不能。
    pub fn reserve_board_number(
        &mut self,
        request_id: &str,
        task_id: TaskId,
        namespace: &str,
        file_max: u64,
    ) -> Result<u64, DbError> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let prior: Option<(String, String, i64)> = tx
            .query_row(
                "SELECT namespace,task_id,number FROM board_numbers WHERE request_id=?1",
                [request_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        if let Some((stored_ns, owner, number)) = prior {
            if stored_ns != namespace || owner != task_id.to_string() {
                return Err(DbError::IdempotencyMismatch);
            }
            return u64::try_from(number)
                .map_err(|_| DbError::Corrupt("negative reserved number".into()));
        }
        let reserved: i64 = tx.query_row(
            "SELECT COALESCE(MAX(number),0) FROM board_numbers WHERE namespace=?1",
            [namespace],
            |r| r.get(0),
        )?;
        let observed = i64::try_from(file_max)
            .map_err(|_| DbError::BoardRejected("number range exhausted"))?;
        let number = reserved
            .max(observed)
            .checked_add(1)
            .ok_or(DbError::BoardRejected("number range exhausted"))?;
        tx.execute(
            "INSERT INTO board_numbers(namespace,number,task_id,request_id) VALUES(?1,?2,?3,?4)",
            params![namespace, number, task_id.to_string(), request_id],
        )?;
        runtime_audit::enqueue(
            &tx,
            &format!("number:{namespace}:{number}"),
            "task.number_reserved",
            serde_json::json!({"task_id":task_id,"namespace":namespace,"number":number}),
        )?;
        tx.commit()?;
        Ok(number as u64)
    }

    /// 列出旧版内联终态正文，供经 Broker staging 的一次性迁移使用。
    pub fn legacy_terminal_attempts(&self) -> Result<Vec<AttemptRecord>, DbError> {
        let mut stmt = self.connection.prepare("SELECT task_id,attempt_id FROM attempts WHERE terminal_json IS NOT NULL AND json_type(terminal_json,'$.artifacts_json')='array'")?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        rows.into_iter()
            .map(|(task, attempt)| {
                self.attempt(parse_task_id(&task)?, parse_attempt_id(&attempt)?)?
                    .ok_or(DbError::NotFound("legacy attempt"))
            })
            .collect()
    }

    /// 只在 Broker 已持久 staging 之后，替换未变化的历史正文。
    pub fn migrate_terminal_pointer(
        &mut self,
        attempt_id: AttemptId,
        expected: &TerminalReservation,
        pointer: &TerminalReservation,
    ) -> Result<(), DbError> {
        if !is_pointer_reservation(pointer) {
            return Err(DbError::BoardRejected("migration pointer has inline data"));
        }
        let changed = self.connection.execute(
            "UPDATE attempts SET terminal_json=?3 WHERE attempt_id=?1 AND terminal_json=?2",
            params![
                attempt_id.to_string(),
                serde_json::to_string(expected)?,
                serde_json::to_string(pointer)?
            ],
        )?;
        if changed != 1 {
            return Err(DbError::IdempotencyMismatch);
        }
        Ok(())
    }
}

pub(super) fn is_pointer_reservation(reservation: &TerminalReservation) -> bool {
    let Some(pointer) = reservation.artifacts_json.as_object() else {
        return false;
    };
    let Some(metadata) = reservation.report_json.as_object() else {
        return false;
    };
    pointer.len() == 2
        && ["id", "sha256"].iter().all(|key| {
            pointer
                .get(*key)
                .and_then(Value::as_str)
                .is_some_and(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
        })
        && metadata.len() <= 7
        && metadata.iter().all(|(key, value)| {
            matches!(
                key.as_str(),
                "format" | "event_id" | "task_id" | "attempt_id" | "fence" | "outcome"
            ) && (value.is_number() || value.as_str().is_some_and(|s| s.len() <= 256))
        })
}
