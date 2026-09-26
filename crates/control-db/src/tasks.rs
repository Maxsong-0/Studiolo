//! 任务表：人写定义的创建、入队、重新确认与查询。
use crate::{
    ControlDb, DbError, SnapshotRecord, TaskRecord, TaskStatus,
    projection_outbox::enqueue_projection,
    snapshots::persist_snapshot,
    types::{parse_attempt_id, parse_task_id},
};
use rusqlite::{OptionalExtension as _, TransactionBehavior, params};
use serde_json::Value;
use studiolo_core::TaskId;

impl ControlDb {
    /// 以 `draft` 状态插入人写任务定义。
    pub fn create_task(
        &mut self,
        task_id: TaskId,
        definition_json: &Value,
        now_ms: i64,
    ) -> Result<(), DbError> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT INTO tasks(task_id, definition_json, status, created_at_ms, updated_at_ms)
             VALUES (?1, ?2, 'draft', ?3, ?3)",
            params![
                task_id.to_string(),
                serde_json::to_string(definition_json)?,
                now_ms
            ],
        )?;
        enqueue_projection(&tx, task_id, &format!("create:{task_id}"))?;
        tx.commit()?;
        Ok(())
    }

    /// 把 draft 任务移入认领队列。
    pub fn queue_task(&mut self, task_id: TaskId, now_ms: i64) -> Result<(), DbError> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = tx.execute(
            "UPDATE tasks SET status = 'queued', updated_at_ms = ?2
             WHERE task_id = ?1 AND status = 'draft'",
            params![task_id.to_string(), now_ms],
        )?;
        if changed == 1 {
            enqueue_projection(&tx, task_id, &format!("queue:{task_id}"))?;
            tx.commit()?;
            Ok(())
        } else {
            drop(tx);
            Err(self.task_transition_error(task_id, "queue")?)
        }
    }

    /// 以确认后的不可变快照把 draft 入队（M0 来自 Run 命令，黑板任务在认领前冻结）。
    pub fn queue_with_snapshot(
        &mut self,
        task_id: TaskId,
        snapshot: &SnapshotRecord,
        now_ms: i64,
    ) -> Result<(), DbError> {
        if snapshot.vault_uuid != self.vault_uuid {
            return Err(DbError::VaultBindingMismatch {
                expected: self.vault_uuid,
                actual: snapshot.vault_uuid,
            });
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        persist_snapshot(&tx, snapshot)?;
        let changed = tx.execute(
            "UPDATE tasks SET status = 'queued', input_snapshot = ?2, updated_at_ms = ?3
             WHERE task_id = ?1 AND status = 'draft'",
            params![task_id.to_string(), &snapshot.snapshot_id, now_ms],
        )?;
        if changed == 0 {
            drop(tx);
            return Err(self.task_transition_error(task_id, "queue")?);
        }
        enqueue_projection(&tx, task_id, &format!("queue:{task_id}"))?;
        tx.commit()?;
        Ok(())
    }

    /// 已确认的输入变化后，取消 queued 任务的认领资格。
    pub fn require_reconfirmation(&mut self, task_id: TaskId, now_ms: i64) -> Result<(), DbError> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = tx.execute(
            "UPDATE tasks SET status = 'needs_reconfirm', updated_at_ms = ?2
             WHERE task_id = ?1 AND status = 'queued'",
            params![task_id.to_string(), now_ms],
        )?;
        if changed == 0 {
            drop(tx);
            return Err(self.task_transition_error(task_id, "require_reconfirmation")?);
        }
        enqueue_projection(&tx, task_id, &format!("reconfirm:{task_id}"))?;
        tx.commit()?;
        Ok(())
    }

    /// 返回最早入队的 queued 任务，但不认领它。
    pub fn next_queued(&self) -> Result<Option<TaskRecord>, DbError> {
        let id: Option<String> = self
            .connection
            .query_row(
                "SELECT task_id FROM tasks WHERE status = 'queued'
                 ORDER BY updated_at_ms, task_id LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        match id {
            Some(value) => self.task(parse_task_id(&value)?),
            None => Ok(None),
        }
    }

    /// 查询单个任务。
    pub fn task(&self, task_id: TaskId) -> Result<Option<TaskRecord>, DbError> {
        type TaskTuple = (
            String,
            String,
            Option<String>,
            Option<String>,
            Option<String>,
            i64,
        );
        let raw: Option<TaskTuple> = self
            .connection
            .query_row(
                "SELECT definition_json, status, active_attempt, last_attempt, input_snapshot, created_at_ms
                 FROM tasks WHERE task_id = ?1",
                [task_id.to_string()],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                },
            )
            .optional()?;
        raw.map(
            |(definition, status, active, last, input_snapshot, created_at_ms)| {
                Ok(TaskRecord {
                    task_id,
                    definition_json: serde_json::from_str(&definition)?,
                    status: TaskStatus::parse(&status)?,
                    active_attempt: active.as_deref().map(parse_attempt_id).transpose()?,
                    last_attempt: last.as_deref().map(parse_attempt_id).transpose()?,
                    input_snapshot,
                    created_at_ms,
                })
            },
        )
        .transpose()
    }

    fn task_transition_error(
        &self,
        task_id: TaskId,
        operation: &'static str,
    ) -> Result<DbError, DbError> {
        match self.task(task_id)? {
            Some(task) => Ok(DbError::InvalidTransition {
                from: task.status.as_str().to_owned(),
                operation,
            }),
            None => Ok(DbError::NotFound("task")),
        }
    }
}
