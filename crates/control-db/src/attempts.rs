//! attempt 的认领、runner 挂接、租约续期、能力判定与重启隔离。
use crate::{
    AttemptPhase, AttemptRecord, ControlDb, DbError, SnapshotRecord, board, lifecycle,
    projection_outbox::enqueue_projection,
    runtime_audit,
    snapshots::persist_snapshot,
    types::{fence_i64, parse_task_id},
};
use rusqlite::{OptionalExtension as _, TransactionBehavior, params};
use serde_json::Value;
use studiolo_core::{AttemptId, Fence, TaskId};

impl ControlDb {
    /// 原子地创建唯一的活动 attempt，并保存快照元数据。
    pub fn claim(
        &mut self,
        task_id: TaskId,
        worker_id: &str,
        snapshot: SnapshotRecord,
        now_ms: i64,
        lease_until_ms: i64,
    ) -> Result<AttemptRecord, DbError> {
        if worker_id.is_empty()
            || lease_until_ms <= now_ms
            || snapshot.vault_uuid != self.vault_uuid
        {
            return Err(DbError::InvalidTransition {
                from: "invalid claim input".to_owned(),
                operation: "claim",
            });
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let state: Option<(String, Option<String>)> = tx
            .query_row(
                "SELECT status, input_snapshot FROM tasks WHERE task_id = ?1",
                [task_id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        match state {
            Some((status, Some(pinned)))
                if status == "queued" && pinned != snapshot.snapshot_id =>
            {
                return Err(DbError::Corrupt(
                    "claim snapshot differs from confirmed Run snapshot".to_owned(),
                ));
            }
            Some((status, _)) if status == "queued" => {}
            Some((other, _)) => {
                return Err(DbError::InvalidTransition {
                    from: other,
                    operation: "claim",
                });
            }
            None => return Err(DbError::NotFound("task")),
        }
        board::require_paths_free(&tx, task_id)?;
        lifecycle::require_retry_ready(&tx, task_id, now_ms)?;
        persist_snapshot(&tx, &snapshot)?;
        let fence_value: i64 = tx.query_row(
            "SELECT next_fence FROM fence_counter WHERE singleton = 1",
            [],
            |row| row.get(0),
        )?;
        tx.execute(
            "UPDATE fence_counter SET next_fence = next_fence + 1 WHERE singleton = 1",
            [],
        )?;
        let fence = Fence::new(
            u64::try_from(fence_value)
                .map_err(|_| DbError::Corrupt("invalid fence counter".to_owned()))?,
        )
        .ok_or_else(|| DbError::Corrupt("zero fence".to_owned()))?;
        let attempt_id = AttemptId::new();
        tx.execute(
            "INSERT INTO attempts(
                 attempt_id, task_id, fence, phase, worker_id, snapshot_id,
                 lease_until_ms, created_at_ms, updated_at_ms)
             VALUES (?1, ?2, ?3, 'running', ?4, ?5, ?6, ?7, ?7)",
            params![
                attempt_id.to_string(),
                task_id.to_string(),
                fence_value,
                worker_id,
                &snapshot.snapshot_id,
                lease_until_ms,
                now_ms
            ],
        )?;
        tx.execute(
            "UPDATE tasks SET status = 'running', active_attempt = ?2,
                 input_snapshot = ?3, updated_at_ms = ?4
             WHERE task_id = ?1",
            params![
                task_id.to_string(),
                attempt_id.to_string(),
                &snapshot.snapshot_id,
                now_ms
            ],
        )?;
        tx.execute(
            "INSERT INTO attempt_progress(attempt_id,last_progress_ms) VALUES(?1,?2)",
            params![attempt_id.to_string(), now_ms],
        )?;
        runtime_audit::enqueue(
            &tx,
            &format!("claim:{attempt_id}"),
            "task.claim",
            serde_json::json!({"task_id":task_id,"attempt_id":attempt_id,"fence":fence.get(),"by_instance":worker_id}),
        )?;
        if tx.query_row(
            "SELECT COUNT(*) FROM board_tasks WHERE task_id=?1 AND handoff_json IS NOT NULL",
            [task_id.to_string()],
            |r| r.get::<_, i64>(0),
        )? != 0
        {
            runtime_audit::enqueue(
                &tx,
                &format!("handoff-accepted:{attempt_id}"),
                "handoff.accepted",
                serde_json::json!({"task_id":task_id,"attempt_id":attempt_id,"by_instance":worker_id}),
            )?;
        }
        enqueue_projection(&tx, task_id, &format!("claim:{attempt_id}"))?;
        tx.commit()?;
        Ok(AttemptRecord {
            attempt_id,
            task_id,
            fence,
            phase: AttemptPhase::Running,
            worker_id: worker_id.to_owned(),
            snapshot_id: snapshot.snapshot_id,
            lease_until_ms,
            runner_ref_json: None,
            terminal: None,
        })
    }

    /// 启动后挂接防 PID 复用的 runner 引用。
    pub fn attach_runner(
        &mut self,
        task_id: TaskId,
        attempt_id: AttemptId,
        fence: Fence,
        runner_ref_json: &Value,
        now_ms: i64,
    ) -> Result<(), DbError> {
        let changed = self.connection.execute(
            "UPDATE attempts SET runner_ref_json = ?4, updated_at_ms = ?5
             WHERE task_id = ?1 AND attempt_id = ?2 AND fence = ?3 AND phase = 'running' AND lease_until_ms > ?5",
            params![
                task_id.to_string(),
                attempt_id.to_string(),
                fence_i64(fence)?,
                serde_json::to_string(runner_ref_json)?,
                now_ms
            ],
        )?;
        if changed == 1 {
            Ok(())
        } else {
            Err(DbError::FenceMismatch)
        }
    }

    /// 核对守卫后续期活动 attempt 的租约。
    pub fn heartbeat(
        &mut self,
        task_id: TaskId,
        attempt_id: AttemptId,
        fence: Fence,
        lease_until_ms: i64,
        now_ms: i64,
    ) -> Result<(), DbError> {
        if lease_until_ms <= now_ms {
            return Err(DbError::FenceMismatch);
        }
        let changed = self.connection.execute(
            "UPDATE attempts SET lease_until_ms = ?4, updated_at_ms = ?5
             WHERE task_id = ?1 AND attempt_id = ?2 AND fence = ?3 AND phase = 'running' AND lease_until_ms > ?5",
            params![
                task_id.to_string(),
                attempt_id.to_string(),
                fence_i64(fence)?,
                lease_until_ms,
                now_ms
            ],
        )?;
        if changed == 1 {
            Ok(())
        } else {
            Err(DbError::FenceMismatch)
        }
    }

    /// 返回 Gateway 能力此刻是否有效。
    pub fn is_attempt_active(
        &self,
        task_id: TaskId,
        attempt_id: AttemptId,
        fence: Fence,
    ) -> Result<bool, DbError> {
        self.is_attempt_active_at(
            task_id,
            attempt_id,
            fence,
            jiff::Timestamp::now().as_millisecond(),
        )
    }

    /// 只把遗留的 `claimed`/`running` attempt 标为 abandoned；`committing` 留待前滚。
    pub fn abandon_inflight(&mut self, now_ms: i64) -> Result<usize, DbError> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let task_ids = {
            let mut statement = tx.prepare(
                "SELECT DISTINCT task_id FROM attempts
                 WHERE phase IN ('claimed', 'running')",
            )?;
            let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        let changed = tx.execute(
            "UPDATE attempts SET phase = 'abandoned', updated_at_ms = ?1
             WHERE phase IN ('claimed', 'running')",
            [now_ms],
        )?;
        for task_id in task_ids {
            tx.execute(
                "UPDATE tasks SET status = 'abandoned', last_attempt = active_attempt,
                     active_attempt = NULL, updated_at_ms = ?2
                 WHERE task_id = ?1 AND status != 'committing'",
                params![task_id, now_ms],
            )?;
            enqueue_projection(
                &tx,
                parse_task_id(&task_id)?,
                &format!("abandoned:{task_id}"),
            )?;
        }
        tx.commit()?;
        Ok(changed)
    }
}
