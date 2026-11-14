//! 产物批准绑定原 attempt 身份，追加后不能改写或删除。
use crate::DbError;
use rusqlite::Transaction;

pub(crate) fn migrate(tx: &Transaction<'_>) -> Result<(), DbError> {
    tx.execute_batch(
        "CREATE UNIQUE INDEX IF NOT EXISTS artifact_attempt_identity
             ON attempts(task_id,attempt_id,fence,snapshot_id);
         CREATE TABLE IF NOT EXISTS artifact_policies (
             attempt_id TEXT PRIMARY KEY,
             task_id TEXT NOT NULL,
             fence INTEGER NOT NULL CHECK(fence>0),
             snapshot_id TEXT NOT NULL,
             provenance_json TEXT NOT NULL CHECK(octet_length(provenance_json) BETWEEN 1 AND 4096 AND json_valid(provenance_json)),
             approval_event_id TEXT NOT NULL UNIQUE REFERENCES runtime_audit_outbox(event_id),
             approved_at_ms INTEGER NOT NULL CHECK(approved_at_ms>=0),
             FOREIGN KEY(task_id,attempt_id,fence,snapshot_id)
                 REFERENCES attempts(task_id,attempt_id,fence,snapshot_id));
         CREATE TRIGGER IF NOT EXISTS artifact_policy_no_update BEFORE UPDATE ON artifact_policies
             BEGIN SELECT RAISE(ABORT,'artifact policy is immutable'); END;
         CREATE TRIGGER IF NOT EXISTS artifact_policy_no_delete BEFORE DELETE ON artifact_policies
             BEGIN SELECT RAISE(ABORT,'artifact policy is immutable'); END;
         CREATE TRIGGER IF NOT EXISTS artifact_policy_before_launch BEFORE INSERT ON artifact_policies
             WHEN NOT EXISTS(SELECT 1 FROM attempts a JOIN tasks t USING(task_id)
                 WHERE a.task_id=NEW.task_id AND a.attempt_id=NEW.attempt_id AND a.fence=NEW.fence
                 AND a.snapshot_id=NEW.snapshot_id AND a.phase='running' AND t.status='running'
                 AND t.active_attempt=a.attempt_id AND a.runner_ref_json IS NULL
                 AND a.created_at_ms<=NEW.approved_at_ms AND a.lease_until_ms>NEW.approved_at_ms)
             BEGIN SELECT RAISE(ABORT,'artifact policy requires original unlaunched attempt'); END;",
    )?;
    Ok(())
}
