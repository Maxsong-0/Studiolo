//! 编号预留：一个写事务内取受信任扫描结果与已持久预留的较大者加一，按 request_id 幂等。
use crate::{ControlDb, DbError, runtime_audit};
use rusqlite::{Connection, OptionalExtension as _, TransactionBehavior, params};
use studiolo_core::TaskId;

impl ControlDb {
    /// 这个请求以前预留过的号，只读；没预留过是 `None`，命名空间或工作项对不上报 `IdempotencyMismatch`。
    /// TaskEngine 重放先查它，不必再扫库：库里的范围目录后来没了，重放也拿回原号（BUG-038）。
    pub fn board_number_replay(
        &self,
        request_id: &str,
        task_id: TaskId,
        namespace: &str,
    ) -> Result<Option<u64>, DbError> {
        prior_number(&self.connection, request_id, task_id, namespace)
    }

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
        if let Some(number) = prior_number(&tx, request_id, task_id, namespace)? {
            return Ok(number);
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

/// 同一 request_id 只认第一次的命名空间和工作项；预留和只读重放共用这一份判断。
fn prior_number(
    db: &Connection,
    request_id: &str,
    task_id: TaskId,
    namespace: &str,
) -> Result<Option<u64>, DbError> {
    let prior: Option<(String, String, i64)> = db
        .query_row(
            "SELECT namespace,task_id,number FROM board_numbers WHERE request_id=?1",
            [request_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let Some((stored_ns, owner, number)) = prior else {
        return Ok(None);
    };
    if stored_ns != namespace || owner != task_id.to_string() {
        return Err(DbError::IdempotencyMismatch);
    }
    u64::try_from(number)
        .map(Some)
        .map_err(|_| DbError::Corrupt("negative reserved number".into()))
}
