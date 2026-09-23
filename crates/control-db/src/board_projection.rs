//! SQLite 只存投影哈希；生成文本与人写文本都留在 Broker。
use super::*;
impl ControlDb {
    /// 读取最近一次成功安装的系统块指纹。
    pub fn board_projection_hash(&self, path: &str) -> Result<Option<String>, DbError> {
        Ok(self
            .connection
            .query_row(
                "SELECT block_hash FROM board_projection_state WHERE path=?1",
                [path],
                |r| r.get(0),
            )
            .optional()?)
    }
    /// 返回时点黑板视图覆盖到的最新投影序号。
    pub fn board_projection_sequence(&self) -> Result<i64, DbError> {
        Ok(self.connection.query_row(
            "SELECT COALESCE(MAX(sequence),0) FROM projection_outbox",
            [],
            |r| r.get(0),
        )?)
    }
    /// Broker 批量发布成功后，只确认本次观察到（序号不超过 `through`）的事件。
    pub fn record_board_projection(
        &mut self,
        path: &str,
        hash: &str,
        project: &str,
        through: i64,
        preserved: Option<&str>,
    ) -> Result<(), DbError> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute("INSERT INTO board_projection_state(path,block_hash) VALUES(?1,?2) ON CONFLICT(path) DO UPDATE SET block_hash=excluded.block_hash",params![path,hash])?;
        tx.execute("UPDATE projection_outbox SET delivered_at_ms=0 WHERE delivered_at_ms IS NULL AND sequence<=?1 AND task_id IN(SELECT b.task_id FROM board_tasks b JOIN tasks t USING(task_id) WHERE json_extract(t.definition_json,'$.project')=?2)",params![through,project])?;
        if let Some(observed) = preserved {
            runtime_audit::enqueue(
                &tx,
                &format!("board-preserved:{path}:{through}:{observed}:{hash}"),
                "task.board_out_of_band_preserved",
                serde_json::json!({"path":path,"project":project,"block_hash":hash,"observed_hash":observed}),
            )?;
        }
        tx.commit()?;
        Ok(())
    }
}
