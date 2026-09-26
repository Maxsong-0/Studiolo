//! 任务页投影 outbox：事务内入队、列出待投递与确认投递。
use crate::{ControlDb, DbError, OutboxRecord, types::parse_task_id};
use rusqlite::{Transaction, params};
use studiolo_core::TaskId;

impl ControlDb {
    /// 返回尚未投递的任务页投影。
    pub fn pending_outbox(&self) -> Result<Vec<OutboxRecord>, DbError> {
        let mut statement = self.connection.prepare(
            "SELECT sequence, task_id, payload_json FROM projection_outbox
             WHERE delivered_at_ms IS NULL ORDER BY sequence",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        let mut records = Vec::new();
        for row in rows {
            let (sequence, task_id, payload) = row?;
            records.push(OutboxRecord {
                sequence,
                task_id: parse_task_id(&task_id)?,
                payload_json: serde_json::from_str(&payload)?,
            });
        }
        Ok(records)
    }

    /// Broker 确认后把一条投影标记为已投递。
    pub fn mark_outbox_delivered(&mut self, sequence: i64, now_ms: i64) -> Result<(), DbError> {
        let changed = self.connection.execute(
            "UPDATE projection_outbox SET delivered_at_ms = ?2
             WHERE sequence = ?1 AND delivered_at_ms IS NULL",
            params![sequence, now_ms],
        )?;
        if changed == 1 {
            Ok(())
        } else {
            Err(DbError::NotFound("pending outbox record"))
        }
    }
}

pub(super) fn enqueue_projection(
    tx: &Transaction<'_>,
    task_id: TaskId,
    event_key: &str,
) -> Result<(), DbError> {
    type ProjectionTuple = (String, Option<String>, Option<String>, Option<String>);
    let (status, active, last, snapshot): ProjectionTuple = tx.query_row(
        "SELECT status, active_attempt, last_attempt, input_snapshot FROM tasks WHERE task_id = ?1",
        [task_id.to_string()],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    let payload = serde_json::json!({
        "task_id": task_id.to_string(), "status": status,
        "active_attempt": active, "last_attempt": last, "input_snapshot": snapshot,
    });
    tx.execute(
        "INSERT OR IGNORE INTO projection_outbox(event_key, task_id, payload_json) VALUES (?1, ?2, ?3)",
        params![event_key, task_id.to_string(), serde_json::to_string(&payload)?],
    )?;
    Ok(())
}
