//! attempt 的只读查询：单条、按任务历史与按阶段列举。
use super::*;

impl ControlDb {
    /// 列出已撤销、取消仍在等待 runner 隔离的 attempt。
    pub fn cancelling_attempts(&self) -> Result<Vec<AttemptRecord>, DbError> {
        self.attempts_with_phase(AttemptPhase::CancelRequested)
    }

    /// 列出对账时须在没有 runner 的情况下前滚的预留。
    pub fn committing_attempts(&self) -> Result<Vec<AttemptRecord>, DbError> {
        self.attempts_with_phase(AttemptPhase::Committing)
    }

    /// 按创建时间/ID 顺序列出某任务的全部历史 attempt，供受信任的隔离维护使用。
    /// 省略终态预留，历史内联报告正文因此不会经此视图外泄。
    pub fn attempts_for_task(&self, task: TaskId) -> Result<Vec<AttemptRecord>, DbError> {
        let mut statement = self.connection.prepare(
            "SELECT attempt_id FROM attempts WHERE task_id=?1 ORDER BY created_at_ms,attempt_id",
        )?;
        let ids = statement
            .query_map([task.to_string()], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        ids.into_iter()
            .map(|id| {
                let mut attempt = self
                    .attempt(task, parse_attempt_id(&id)?)?
                    .ok_or(DbError::NotFound("historical attempt"))?;
                attempt.terminal = None;
                Ok(attempt)
            })
            .collect()
    }

    /// 查询单个 attempt，含其终态前滚预留。
    pub fn attempt(
        &self,
        task_id: TaskId,
        attempt_id: AttemptId,
    ) -> Result<Option<AttemptRecord>, DbError> {
        type AttemptTuple = (
            i64,
            String,
            String,
            String,
            i64,
            Option<String>,
            Option<String>,
        );
        let row: Option<AttemptTuple> = self
            .connection
            .query_row(
                "SELECT fence, phase, worker_id, snapshot_id, lease_until_ms,
                        runner_ref_json, terminal_json
                 FROM attempts WHERE task_id = ?1 AND attempt_id = ?2",
                params![task_id.to_string(), attempt_id.to_string()],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                    ))
                },
            )
            .optional()?;
        row.map(
            |(fence, phase, worker_id, snapshot_id, lease, runner, terminal)| {
                Ok(AttemptRecord {
                    attempt_id,
                    task_id,
                    fence: Fence::new(
                        u64::try_from(fence)
                            .map_err(|_| DbError::Corrupt("invalid attempt fence".to_owned()))?,
                    )
                    .ok_or_else(|| DbError::Corrupt("zero attempt fence".to_owned()))?,
                    phase: AttemptPhase::parse(&phase)?,
                    worker_id,
                    snapshot_id,
                    lease_until_ms: lease,
                    runner_ref_json: runner
                        .map(|value| serde_json::from_str(&value))
                        .transpose()?,
                    terminal: terminal
                        .map(|value| serde_json::from_str(&value))
                        .transpose()?,
                })
            },
        )
        .transpose()
    }

    fn attempts_with_phase(&self, wanted: AttemptPhase) -> Result<Vec<AttemptRecord>, DbError> {
        type AttemptTuple = (
            String,
            String,
            i64,
            String,
            String,
            String,
            i64,
            Option<String>,
            Option<String>,
        );
        let mut statement = self.connection.prepare(
            "SELECT attempt_id, task_id, fence, phase, worker_id, snapshot_id,
                    lease_until_ms, runner_ref_json, terminal_json
             FROM attempts WHERE phase = ?1 ORDER BY created_at_ms, attempt_id",
        )?;
        let rows = statement.query_map([wanted.as_str()], |row| {
            Ok::<AttemptTuple, rusqlite::Error>((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
                row.get(7)?,
                row.get(8)?,
            ))
        })?;
        let mut attempts = Vec::new();
        for row in rows {
            let (
                attempt_id,
                task_id,
                fence,
                phase,
                worker_id,
                snapshot_id,
                lease,
                runner,
                terminal,
            ) = row?;
            attempts.push(AttemptRecord {
                attempt_id: parse_attempt_id(&attempt_id)?,
                task_id: parse_task_id(&task_id)?,
                fence: Fence::new(
                    u64::try_from(fence)
                        .map_err(|_| DbError::Corrupt("invalid attempt fence".to_owned()))?,
                )
                .ok_or_else(|| DbError::Corrupt("zero attempt fence".to_owned()))?,
                phase: AttemptPhase::parse(&phase)?,
                worker_id,
                snapshot_id,
                lease_until_ms: lease,
                runner_ref_json: runner
                    .map(|value| serde_json::from_str(&value))
                    .transpose()?,
                terminal: terminal
                    .map(|value| serde_json::from_str(&value))
                    .transpose()?,
            });
        }
        Ok(attempts)
    }
}
