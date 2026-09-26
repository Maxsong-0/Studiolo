//! 编号预留：一个写事务内取受信任扫描结果与已持久预留的较大者加一，按 request_id 幂等。
use crate::{ControlDb, DbError, runtime_audit};
use rusqlite::{OptionalExtension as _, TransactionBehavior, params};
use studiolo_core::TaskId;

impl ControlDb {
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
}
