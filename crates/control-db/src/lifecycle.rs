//! 租约时效权威与感知进度的对账；后者有别于启动恢复。
use super::*;

/// 活动 attempt 的单调进度计数。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttemptProgress {
    #[doc = "已尝试的工具调用次数，单调递增。"]
    pub tool_calls: u64,
    #[doc = "编辑次数，单调递增。"]
    pub edits: u64,
    #[doc = "任一进度计数最近一次增长的时间。"]
    pub last_progress_ms: i64,
}

/// 一次租约巡检造成的变更；每条记录都提供持久的审计主体与原因。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeaseReconciliation {
    #[doc = "被撤销的任务与 attempt 标识，附机器可读的原因。"]
    pub abandoned: Vec<(TaskId, AttemptId, String)>,
}

pub(super) fn require_live(
    tx: &Transaction<'_>,
    task: TaskId,
    attempt: AttemptId,
    fence: Fence,
    now: i64,
) -> Result<(), DbError> {
    let active: i64 = tx.query_row("SELECT COUNT(*) FROM attempts a JOIN tasks t USING(task_id) WHERE a.task_id=?1 AND a.attempt_id=?2 AND a.fence=?3 AND a.phase='running' AND t.status='running' AND t.active_attempt=a.attempt_id AND a.lease_until_ms>?4",params![task.to_string(),attempt.to_string(),fence_i64(fence)?,now],|r|r.get(0))?;
    if active != 1 {
        return Err(DbError::FenceMismatch);
    }
    Ok(())
}

pub(super) fn require_retry_ready(
    tx: &Transaction<'_>,
    task: TaskId,
    now: i64,
) -> Result<(), DbError> {
    let blocked: i64 = tx.query_row(
        "SELECT COUNT(*) FROM task_launch_retries WHERE task_id=?1 AND not_before_ms>?2",
        params![task.to_string(), now],
        |r| r.get(0),
    )?;
    if blocked != 0 {
        return Err(DbError::BoardRejected(
            "launch retry backoff has not elapsed",
        ));
    }
    Ok(())
}

impl ControlDb {
    /// Gateway 的权威守卫：过期租约永远不能授权工具调用。
    pub fn is_attempt_active_at(
        &self,
        task: TaskId,
        attempt: AttemptId,
        fence: Fence,
        now: i64,
    ) -> Result<bool, DbError> {
        let active: i64 = self.connection.query_row("SELECT COUNT(*) FROM attempts a JOIN tasks t USING(task_id) WHERE a.task_id=?1 AND a.attempt_id=?2 AND a.fence=?3 AND a.phase='running' AND t.status='running' AND t.active_attempt=a.attempt_id AND a.lease_until_ms>?4",params![task.to_string(),attempt.to_string(),fence_i64(fence)?,now],|r|r.get(0))?;
        Ok(active == 1)
    }

    /// 只续期未过期的租约，并拒绝回退的计数。
    #[allow(clippy::too_many_arguments)]
    pub fn heartbeat_progress(
        &mut self,
        task: TaskId,
        attempt: AttemptId,
        fence: Fence,
        lease_until: i64,
        tool_calls: u64,
        edits: u64,
        now: i64,
    ) -> Result<(), DbError> {
        if lease_until <= now {
            return Err(DbError::FenceMismatch);
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        require_live(&tx, task, attempt, fence, now)?;
        let (tools, prior_edits): (i64, i64) = tx.query_row(
            "SELECT tool_calls,edits FROM attempt_progress WHERE attempt_id=?1",
            [attempt.to_string()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let tool_calls = i64::try_from(tool_calls)
            .map_err(|_| DbError::BoardRejected("tool count exceeds SQLite range"))?;
        let edits = i64::try_from(edits)
            .map_err(|_| DbError::BoardRejected("edit count exceeds SQLite range"))?;
        if tools < 0 || prior_edits < 0 {
            return Err(DbError::Corrupt("negative progress counter".into()));
        }
        if tool_calls < tools || edits < prior_edits {
            return Err(DbError::BoardRejected(
                "progress counters must be monotonic",
            ));
        }
        tx.execute("UPDATE attempt_progress SET tool_calls=?2,edits=?3,last_progress_ms=CASE WHEN tool_calls<?2 OR edits<?3 THEN ?4 ELSE last_progress_ms END WHERE attempt_id=?1",params![attempt.to_string(),tool_calls,edits,now])?;
        tx.execute(
            "UPDATE attempts SET lease_until_ms=?2,updated_at_ms=?3 WHERE attempt_id=?1",
            params![attempt.to_string(), lease_until, now],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// 只返回计数，不含正文或 runner 输出。
    pub fn attempt_progress(&self, attempt: AttemptId) -> Result<Option<AttemptProgress>, DbError> {
        let row:Option<(i64,i64,i64)>=self.connection.query_row("SELECT tool_calls,edits,last_progress_ms FROM attempt_progress WHERE attempt_id=?1",[attempt.to_string()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
        row.map(|(tools, edits, last)| {
            Ok(AttemptProgress {
                tool_calls: u64::try_from(tools)
                    .map_err(|_| DbError::Corrupt("negative tool counter".into()))?,
                edits: u64::try_from(edits)
                    .map_err(|_| DbError::Corrupt("negative edit counter".into()))?,
                last_progress_ms: last,
            })
        })
        .transpose()
    }

    /// 周期巡检保留健康的 running attempt；启动巡检则显式撤销它们。
    pub fn reconcile_leases(
        &mut self,
        now: i64,
        startup: bool,
    ) -> Result<LeaseReconciliation, DbError> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let rows = {
            let mut stmt=tx.prepare("SELECT a.task_id,a.attempt_id,a.phase,a.lease_until_ms,p.last_progress_ms,p.edits,p.cancel_requested_ms,json_extract(t.definition_json,'$.adapter') FROM attempts a JOIN tasks t USING(task_id) JOIN attempt_progress p USING(attempt_id) LEFT JOIN board_tasks b USING(task_id) WHERE a.phase IN ('running','claimed','cancel_requested') ORDER BY COALESCE(b.depth,0) DESC")?;
            stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, i64>(4)?,
                    r.get::<_, i64>(5)?,
                    r.get::<_, Option<i64>>(6)?,
                    r.get::<_, String>(7)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?
        };
        let mut result = LeaseReconciliation::default();
        for (task, attempt, phase, lease, last, edits, cancel, adapter) in rows {
            let reason = if startup {
                Some("restart")
            } else if phase == "cancel_requested" {
                cancel
                    .filter(|start| now >= start.saturating_add(180_000))
                    .map(|_| "cancel_timeout")
            } else if lease <= now {
                Some("lease_expired")
            } else if adapter != "consumer" && edits == 0 && now >= last.saturating_add(600_000) {
                Some("stalled")
            } else {
                None
            };
            let Some(reason) = reason else { continue };
            let task_id = parse_task_id(&task)?;
            let children = board::require_children_terminal(&tx, task_id);
            if let Err(error) = children {
                if !matches!(error, DbError::BoardRejected(_)) {
                    return Err(error);
                }
                tx.execute("UPDATE attempts SET phase='cancel_requested',updated_at_ms=?2 WHERE attempt_id=?1",params![attempt,now])?;
                tx.execute(
                    "UPDATE tasks SET status='cancel_requested',updated_at_ms=?2 WHERE task_id=?1",
                    params![task, now],
                )?;
                tx.execute("UPDATE attempt_progress SET cancel_requested_ms=COALESCE(cancel_requested_ms,?2) WHERE attempt_id=?1",params![attempt,now])?;
                continue;
            }
            tx.execute(
                "UPDATE attempts SET phase='abandoned',updated_at_ms=?2 WHERE attempt_id=?1",
                params![attempt, now],
            )?;
            tx.execute(
                "UPDATE attempt_progress SET terminal_reason=?2 WHERE attempt_id=?1",
                params![attempt, reason],
            )?;
            tx.execute("UPDATE tasks SET status='abandoned',last_attempt=active_attempt,active_attempt=NULL,updated_at_ms=?2 WHERE task_id=?1",params![task,now])?;
            let definition: Option<String> = tx
                .query_row(
                    "SELECT definition_json FROM board_tasks WHERE task_id=?1",
                    [&task],
                    |r| r.get(0),
                )
                .optional()?;
            let acceptance=definition.map(|raw|serde_json::from_str::<BoardDefinition>(&raw)).transpose()?.map(|d|d.acceptance_criteria.into_iter().map(|criterion|serde_json::json!({"criterion":criterion,"result":"unverified","evidence":[]})).collect::<Vec<_>>()).unwrap_or_default();
            let receipt = serde_json::json!({"status":"abandoned","artifacts":[],"conflicts":[],"unlanded":[],"acceptance":acceptance,"remaining":[format!("Attempt abandoned: {reason}")],"elapsed_ms":0,"tool_calls":0,"learned":[],"closeout":null,"system_reason":reason});
            tx.execute(
                "UPDATE board_tasks SET receipt_json=COALESCE(receipt_json,?2) WHERE task_id=?1",
                params![task, serde_json::to_string(&receipt)?],
            )?;
            enqueue_projection(&tx, task_id, &format!("abandoned:{attempt}"))?;
            runtime_audit::enqueue(
                &tx,
                &format!("abandoned:{attempt}"),
                "task.abandoned",
                serde_json::json!({"task_id":task,"attempt_id":attempt,"reason":reason}),
            )?;
            result
                .abandoned
                .push((task_id, parse_attempt_id(&attempt)?, reason.into()));
        }
        tx.commit()?;
        Ok(result)
    }

    /// 只重试首个工具调用前的启动失败：最多两次、带退避，并受 `allow_retry` 与 `max_attempts` 约束。
    #[allow(clippy::too_many_arguments)]
    pub fn launch_failed(
        &mut self,
        task: TaskId,
        attempt: AttemptId,
        fence: Fence,
        allow_retry: bool,
        max_attempts: u32,
        backoff_ms: i64,
        now: i64,
    ) -> Result<bool, DbError> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        require_live(&tx, task, attempt, fence, now)?;
        let calls: i64 = tx.query_row(
            "SELECT tool_calls FROM attempt_progress WHERE attempt_id=?1",
            [attempt.to_string()],
            |r| r.get(0),
        )?;
        if calls != 0 {
            return Err(DbError::BoardRejected("launch failure follows a tool call"));
        }
        board::require_children_terminal(&tx, task)?;
        let retries: u32 = tx
            .query_row(
                "SELECT retries FROM task_launch_retries WHERE task_id=?1",
                [task.to_string()],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or(0);
        let retry = allow_retry && retries < 2 && retries.saturating_add(1) < max_attempts;
        tx.execute(
            "UPDATE attempts SET phase='failed',updated_at_ms=?2 WHERE attempt_id=?1",
            params![attempt.to_string(), now],
        )?;
        tx.execute(
            "UPDATE attempt_progress SET terminal_reason='launch_failure' WHERE attempt_id=?1",
            [attempt.to_string()],
        )?;
        tx.execute("UPDATE tasks SET status=?2,last_attempt=active_attempt,active_attempt=NULL,updated_at_ms=?3 WHERE task_id=?1",params![task.to_string(),if retry {"queued"}else{"failed"},now])?;
        if retry {
            tx.execute("INSERT INTO task_launch_retries(task_id,retries,not_before_ms) VALUES(?1,?2,?3) ON CONFLICT(task_id) DO UPDATE SET retries=excluded.retries,not_before_ms=excluded.not_before_ms",params![task.to_string(),retries+1,now.saturating_add(backoff_ms.max(0))])?;
        }
        runtime_audit::enqueue(
            &tx,
            &format!("launch-failed:{attempt}"),
            "task.launch_failed",
            serde_json::json!({"task_id":task,"attempt_id":attempt,"retry":retry}),
        )?;
        enqueue_projection(&tx, task, &format!("launch-failed:{attempt}"))?;
        tx.commit()?;
        Ok(retry)
    }
}
