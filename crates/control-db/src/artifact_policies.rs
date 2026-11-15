//! 可信启动的持久批准；与原运行时审计在同一事务，Runner 报告没有此入口。
mod schema;
use crate::{ControlDb, DbError, RuntimeAuditRecord, lifecycle, runtime_audit, types::fence_i64};
use rusqlite::{Connection, OptionalExtension as _, TransactionBehavior, params};
pub(super) use schema::migrate;
use serde_json::json;
use studiolo_audit_ledger::AuditEvent;
use studiolo_core::{ArtifactProvenance, AttemptId, Fence, TaskId};

impl ControlDb {
    /// 在首次执行体挂接前持久化完整产物批准；精确重放保留原时间与原审计。
    #[allow(clippy::too_many_arguments)]
    pub fn authorize_artifacts(
        &mut self,
        task: TaskId,
        attempt: AttemptId,
        fence: Fence,
        snapshot_id: &str,
        provenance: &ArtifactProvenance,
        now_ms: i64,
    ) -> Result<(), DbError> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(prior) = read(&tx, task, attempt, fence, snapshot_id)? {
            if prior != *provenance {
                return Err(DbError::IdempotencyMismatch);
            }
            // 重放只确认原记录；不会据此延长 lease、留存期或再允许正文写入。
            tx.commit()?;
            return Ok(());
        }
        lifecycle::require_live(&tx, task, attempt, fence, now_ms)?;
        if !provenance.is_live_at(now_ms) {
            return Err(DbError::BoardRejected("产物留存期已过或时钟无效"));
        }
        let encoded = serde_json::to_string(provenance)?;
        if encoded.len() > 4096 {
            return Err(DbError::BoardRejected("产物批准超过元数据上限"));
        }
        let event = event(task, attempt, fence, snapshot_id, provenance, now_ms);
        // 在事务内先验证实际审计事件，不能在库提交后才发现无法投递。
        event.audit_event()?;
        runtime_audit::enqueue(&tx, &event.event_id, &event.action, event.payload)?;
        tx.execute("INSERT INTO artifact_policies(attempt_id,task_id,fence,snapshot_id,provenance_json,approval_event_id,approved_at_ms) VALUES(?1,?2,?3,?4,?5,?6,?7)",
            params![attempt.to_string(),task.to_string(),fence_i64(fence)?,snapshot_id,encoded,event.event_id,now_ms])?;
        // INSERT OR IGNORE 的审计幂等键若已有其他事实，整笔批准回滚。
        read(&tx, task, attempt, fence, snapshot_id)?.ok_or_else(invalid)?;
        tx.commit()?;
        Ok(())
    }

    /// 读取原批准并核对原 attempt 与持久审计事实；不续期、不替旧 attempt 补默认值。
    pub fn artifact_provenance(
        &self,
        task: TaskId,
        attempt: AttemptId,
        fence: Fence,
    ) -> Result<Option<ArtifactProvenance>, DbError> {
        let row = self
            .attempt(task, attempt)?
            .ok_or(DbError::NotFound("attempt"))?;
        if row.fence != fence {
            return Err(DbError::FenceMismatch);
        }
        read(&self.connection, task, attempt, fence, &row.snapshot_id)
    }

    /// 原批准对应的账本事件。没有批准时返回空，不补造完整范围。
    pub fn artifact_approval_event(
        &self,
        task: TaskId,
        attempt: AttemptId,
        fence: Fence,
    ) -> Result<Option<AuditEvent>, DbError> {
        let row = self
            .attempt(task, attempt)?
            .ok_or(DbError::NotFound("attempt"))?;
        if row.fence != fence {
            return Err(DbError::FenceMismatch);
        }
        let Some(provenance) = read(&self.connection, task, attempt, fence, &row.snapshot_id)?
        else {
            return Ok(None);
        };
        let approved_at: Option<i64> = self
            .connection
            .query_row(
                "SELECT approved_at_ms FROM artifact_policies WHERE attempt_id=?1",
                [attempt.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        let approved_at = approved_at.ok_or_else(invalid)?;
        Ok(Some(
            event(
                task,
                attempt,
                fence,
                &row.snapshot_id,
                &provenance,
                approved_at,
            )
            .audit_event()?,
        ))
    }
}

fn event(
    task: TaskId,
    attempt: AttemptId,
    fence: Fence,
    snapshot: &str,
    provenance: &ArtifactProvenance,
    now: i64,
) -> RuntimeAuditRecord {
    RuntimeAuditRecord {
        event_id: format!("artifact-policy:{attempt}"),
        action: "task.artifacts.authorized".into(),
        payload: json!({"schema":1,"task_id":task,"attempt_id":attempt,"fence":fence,"snapshot_id":snapshot,"provenance":provenance,"approved_at_ms":now}),
    }
}

fn read(
    db: &Connection,
    task: TaskId,
    attempt: AttemptId,
    fence: Fence,
    snapshot: &str,
) -> Result<Option<ArtifactProvenance>, DbError> {
    type Row = (String, i64, String, String, String, i64);
    let row: Option<Row> = db.query_row("SELECT task_id,fence,snapshot_id,CASE WHEN octet_length(provenance_json) BETWEEN 1 AND 4096 THEN provenance_json END,approval_event_id,approved_at_ms FROM artifact_policies WHERE attempt_id=?1",
        [attempt.to_string()], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).optional()?;
    let Some((stored_task, stored_fence, stored_snapshot, encoded, event_id, approved_at)) = row
    else {
        return Ok(None);
    };
    if stored_task != task.to_string()
        || stored_fence != fence_i64(fence)?
        || stored_snapshot != snapshot
    {
        return Err(DbError::IdempotencyMismatch);
    }
    let provenance: ArtifactProvenance = serde_json::from_str(&encoded)?;
    if !provenance.is_live_at(approved_at) {
        return Err(invalid());
    }
    let expected = event(task, attempt, fence, snapshot, &provenance, approved_at);
    if event_id != expected.event_id {
        return Err(invalid());
    }
    let audit: Option<(String, String)> = db.query_row("SELECT action,CASE WHEN octet_length(payload_json) BETWEEN 1 AND 8192 THEN payload_json END FROM runtime_audit_outbox WHERE event_id=?1", [event_id], |r| Ok((r.get(0)?,r.get(1)?))).optional()?;
    let Some((action, payload)) = audit else {
        return Err(invalid());
    };
    if action != expected.action
        || serde_json::from_str::<serde_json::Value>(&payload)? != expected.payload
    {
        return Err(invalid());
    }
    expected.audit_event()?;
    Ok(Some(provenance))
}

pub(super) fn validate(db: &Connection) -> Result<(), DbError> {
    let orphan: i64 = db.query_row("SELECT count(*) FROM runtime_audit_outbox o WHERE o.action='task.artifacts.authorized' AND NOT EXISTS(SELECT 1 FROM artifact_policies p WHERE p.approval_event_id=o.event_id)", [], |r| r.get(0))?;
    if orphan != 0 {
        return Err(invalid());
    }
    let mut statement = db.prepare(
        "SELECT task_id,attempt_id,fence,snapshot_id FROM artifact_policies ORDER BY attempt_id",
    )?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let task = crate::types::parse_task_id(&row.get::<_, String>(0)?)?;
        let attempt = crate::types::parse_attempt_id(&row.get::<_, String>(1)?)?;
        let fence = Fence::new(u64::try_from(row.get::<_, i64>(2)?).map_err(|_| invalid())?)
            .ok_or_else(invalid)?;
        read(db, task, attempt, fence, &row.get::<_, String>(3)?)?.ok_or_else(invalid)?;
    }
    Ok(())
}
fn invalid() -> DbError {
    DbError::Corrupt("原产物批准与审计事实不一致".into())
}
