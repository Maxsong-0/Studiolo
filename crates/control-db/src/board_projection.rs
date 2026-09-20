//! Only projection hashes live in SQLite; generated and human text remains in the Broker.
use super::*;
impl ControlDb {
    /// Reads the last successfully installed system block fingerprint.
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
    /// Returns the latest projection sequence covered by a point-in-time board view.
    pub fn board_projection_sequence(&self) -> Result<i64, DbError> {
        Ok(self.connection.query_row(
            "SELECT COALESCE(MAX(sequence),0) FROM projection_outbox",
            [],
            |r| r.get(0),
        )?)
    }
    /// Acknowledges only the observed events after successful Broker batch publication.
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
