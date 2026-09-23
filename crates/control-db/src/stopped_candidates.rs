//! 停止候选与终态共用 T1/T2，其取消结果在预留时即冻结。
use super::*;

impl ControlDb {
    /// T1 只在权威取消宽限期内接受停止态 staging 指针。
    pub fn reserve_board_stopped_candidates(
        &mut self,
        task: TaskId,
        attempt: AttemptId,
        fence: Fence,
        reservation: &TerminalReservation,
        now: i64,
    ) -> Result<(), DbError> {
        self.reserve_terminal_kind(
            task,
            attempt,
            fence,
            reservation,
            now,
            TerminalKind::BoardStopped,
        )
    }

    /// T2 原子记录已核验的停止回执、已提交的产物与取消结果。
    pub fn finalize_board_stopped_candidates(
        &mut self,
        task: TaskId,
        attempt: AttemptId,
        fence: Fence,
        artifacts: &Value,
        receipt: &Value,
        now: i64,
    ) -> Result<(), DbError> {
        self.finalize_terminal_kind(
            task,
            attempt,
            fence,
            TaskStatus::Cancelled,
            Some(artifacts),
            Some(receipt),
            now,
        )
    }
}

pub(super) fn require_cancel_window(
    tx: &Transaction<'_>,
    task: TaskId,
    attempt: AttemptId,
    fence: Fence,
    now: i64,
) -> Result<(), DbError> {
    let requested: Option<i64> = tx.query_row(
        "SELECT p.cancel_requested_ms FROM attempts a JOIN tasks t USING(task_id) JOIN attempt_progress p USING(attempt_id) WHERE a.task_id=?1 AND a.attempt_id=?2 AND a.fence=?3 AND a.phase='cancel_requested' AND t.status='cancel_requested' AND t.active_attempt=a.attempt_id",
        params![task.to_string(),attempt.to_string(),fence_i64(fence)?], |r|r.get(0),
    ).optional()?.flatten();
    if requested.is_none_or(|start| now < start || now >= start.saturating_add(180_000)) {
        return Err(DbError::BoardRejected(
            "stopped candidates require the active cancellation window",
        ));
    }
    Ok(())
}

pub(super) fn validate_finalization(
    tx: &Transaction<'_>,
    task: TaskId,
    attempt: AttemptId,
    status: TaskStatus,
    artifacts: Option<&Value>,
    receipt: Option<&Value>,
) -> Result<(), DbError> {
    let terminal: Option<String> = tx
        .query_row(
            "SELECT terminal_json FROM attempts WHERE task_id=?1 AND attempt_id=?2",
            params![task.to_string(), attempt.to_string()],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    let reservation = terminal
        .map(|raw| serde_json::from_str::<TerminalReservation>(&raw))
        .transpose()?;
    let stopped = reservation
        .as_ref()
        .is_some_and(|r| r.report_json.get("outcome").and_then(Value::as_str) == Some("stopped"));
    if stopped != receipt.is_some() {
        return Err(DbError::BoardRejected(
            "terminal outcome differs from its frozen reservation",
        ));
    }
    let Some(receipt) = receipt else {
        return Ok(());
    };
    if status != TaskStatus::Cancelled
        || artifacts.is_none()
        || receipt.get("status").and_then(Value::as_str) != Some("stopped")
        || !receipt
            .get("conflicts")
            .and_then(Value::as_array)
            .is_some_and(Vec::is_empty)
        || !receipt.get("closeout").is_some_and(Value::is_object)
    {
        return Err(DbError::BoardRejected("invalid stopped candidate receipt"));
    }
    let row: Option<(String,Option<String>,String)> = tx.query_row(
        "SELECT b.definition_json,b.receipt_json,json_extract(t.definition_json,'$.adapter') FROM board_tasks b JOIN tasks t USING(task_id) WHERE b.task_id=?1 AND COALESCE(t.active_attempt,t.last_attempt)=?2",
        params![task.to_string(),attempt.to_string()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
    ).optional()?;
    let (definition, prior, adapter) = row.ok_or(DbError::FenceMismatch)?;
    if adapter == "consumer" {
        return Err(DbError::BoardRejected("consumer has no runner candidates"));
    }
    let definition: BoardDefinition = serde_json::from_str(&definition)?;
    let assessments = receipt
        .get("acceptance")
        .and_then(Value::as_array)
        .ok_or(DbError::BoardRejected("missing stopped assessments"))?;
    if assessments.len() != definition.acceptance_criteria.len()
        || assessments
            .iter()
            .zip(&definition.acceptance_criteria)
            .any(|(a, c)| {
                a.get("criterion").and_then(Value::as_str) != Some(c.as_str())
                    || !matches!(
                        a.get("result").and_then(Value::as_str),
                        Some("pass" | "fail" | "unverified")
                    )
            })
        || assessments
            .iter()
            .all(|a| a.get("result").and_then(Value::as_str) == Some("pass"))
    {
        return Err(DbError::BoardRejected(
            "stopped assessments must preserve incomplete work",
        ));
    }
    if let Some(prior) = prior
        && serde_json::from_str::<Value>(&prior)? != *receipt
    {
        return Err(DbError::IdempotencyMismatch);
    }
    board::require_children_terminal(tx, task)?;
    Ok(())
}
