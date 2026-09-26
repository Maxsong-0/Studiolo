//! 候选保留只是证据索引；T1/T2 仍是唯一的发布状态机。
use crate::{
    ControlDb, DbError, TerminalReservation, board_completion, runtime_audit,
    types::{fence_i64, parse_attempt_id},
};
use rusqlite::{TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use studiolo_core::{AttemptId, Fence, TaskId};
/// 保留下来的内容面 staging 指针，含在 T1 之前被拒绝或取消的候选。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RunnerCandidateStage {
    /// 产生该候选的 attempt。
    pub attempt_id: AttemptId,
    /// 该 attempt 提供的 fence。
    pub fence: Fence,
    /// 只含 Broker staging 标识与摘要，绝不含候选字节。
    pub pointer: Value,
    /// 保留该候选的原因；保留本身不代表任务已成功完成。
    pub reason: String,
}
impl ControlDb {
    /// 保存已核验 attempt 的 staging 指针，但不因此授予工具或完成权限。
    pub fn retain_runner_candidate(
        &mut self,
        task: TaskId,
        attempt: AttemptId,
        fence: Fence,
        reservation: &TerminalReservation,
        reason: &str,
    ) -> Result<(), DbError> {
        if !board_completion::is_pointer_reservation(reservation) {
            return Err(DbError::BoardRejected(
                "candidate retention requires a staging pointer",
            ));
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let matches:i64=tx.query_row("SELECT COUNT(*) FROM attempts a JOIN tasks t USING(task_id) JOIN board_tasks b USING(task_id) WHERE a.task_id=?1 AND a.attempt_id=?2 AND a.fence=?3 AND COALESCE(t.active_attempt,t.last_attempt)=a.attempt_id AND json_extract(t.definition_json,'$.adapter')!='consumer'",params![task.to_string(),attempt.to_string(),fence_i64(fence)?],|r|r.get(0))?;
        if matches != 1 {
            return Err(DbError::FenceMismatch);
        }
        let pointer = serde_json::to_string(&reservation.artifacts_json)?;
        tx.execute("INSERT INTO runner_candidate_stages(task_id,attempt_id,fence,pointer_json,reason) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(attempt_id,pointer_json) DO UPDATE SET reason=excluded.reason",params![task.to_string(),attempt.to_string(),fence_i64(fence)?,pointer,reason])?;
        let digest = reservation.artifacts_json["sha256"]
            .as_str()
            .ok_or(DbError::BoardRejected("missing staging hash"))?;
        runtime_audit::enqueue(
            &tx,
            &format!("candidate:{attempt}:{digest}:{reason}"),
            "task.runner_candidate_retained",
            serde_json::json!({"task_id":task,"attempt_id":attempt,"stage_hash":digest,"reason":reason}),
        )?;
        tx.commit()?;
        Ok(())
    }
    /// 列出全部候选的持久元数据引用，含未成功的提交。
    pub fn runner_candidate_stages(
        &self,
        task: TaskId,
    ) -> Result<Vec<RunnerCandidateStage>, DbError> {
        let mut statement=self.connection.prepare("SELECT attempt_id,fence,pointer_json,reason FROM runner_candidate_stages WHERE task_id=?1 ORDER BY attempt_id,pointer_json")?;
        let rows = statement.query_map([task.to_string()], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?;
        let mut result = Vec::new();
        for row in rows {
            let (attempt, fence, pointer, reason) = row?;
            result.push(RunnerCandidateStage {
                attempt_id: parse_attempt_id(&attempt)?,
                fence: Fence::new(
                    u64::try_from(fence)
                        .map_err(|_| DbError::Corrupt("negative candidate fence".into()))?,
                )
                .ok_or(DbError::Corrupt("zero candidate fence".into()))?,
                pointer: serde_json::from_str(&pointer)?,
                reason,
            });
        }
        Ok(result)
    }
}
