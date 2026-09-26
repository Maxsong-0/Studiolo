//! 已落定终态的 Broker 回执读取与审计结果投递确认。
use crate::{
    ControlDb, DbError, PendingTerminalAudit, TaskStatus,
    types::{parse_attempt_id, parse_task_id},
};
use rusqlite::{OptionalExtension as _, params};
use serde_json::Value;
use studiolo_core::{AttemptId, TaskId};

impl ControlDb {
    /// 读取已提交的 Broker 回执，不触碰规范产物。
    pub fn artifact_receipt(
        &self,
        task_id: TaskId,
        attempt_id: AttemptId,
    ) -> Result<Option<Value>, DbError> {
        let receipt: Option<String> = self
            .connection
            .query_row(
                "SELECT c.receipt_json FROM artifact_commits c JOIN attempts a USING(attempt_id)
             WHERE a.task_id = ?1 AND a.attempt_id = ?2",
                params![task_id.to_string(), attempt_id.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        receipt
            .map(|value| serde_json::from_str(&value).map_err(DbError::from))
            .transpose()
    }

    /// 只读取 AuditLedger 尚未确认的持久终态结果。
    pub fn pending_terminal_audits(&self) -> Result<Vec<PendingTerminalAudit>, DbError> {
        let mut statement = self.connection.prepare(
            "SELECT a.task_id, o.attempt_id, o.status, c.receipt_json
             FROM terminal_audit_outbox o JOIN attempts a USING(attempt_id)
             JOIN artifact_commits c USING(attempt_id)
             WHERE o.delivered_at_ms IS NULL ORDER BY o.attempt_id",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?;
        let mut outcomes = Vec::new();
        for row in rows {
            let (task, attempt, status, receipt) = row?;
            let attempt = self
                .attempt(parse_task_id(&task)?, parse_attempt_id(&attempt)?)?
                .ok_or(DbError::NotFound("pending audit attempt"))?;
            outcomes.push(PendingTerminalAudit {
                attempt,
                status: TaskStatus::parse(&status)?,
                receipt_json: serde_json::from_str(&receipt)?,
            });
        }
        Ok(outcomes)
    }

    /// 只在 AuditLedger 追加成功后确认一条结果。
    pub fn mark_terminal_audit_delivered(
        &mut self,
        attempt_id: AttemptId,
        now_ms: i64,
    ) -> Result<(), DbError> {
        let changed = self.connection.execute(
            "UPDATE terminal_audit_outbox SET delivered_at_ms = COALESCE(delivered_at_ms, ?2)
             WHERE attempt_id = ?1",
            params![attempt_id.to_string(), now_ms],
        )?;
        if changed == 0 {
            return Err(DbError::NotFound("pending terminal audit"));
        }
        Ok(())
    }
}
