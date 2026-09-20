//! Incremental v3 metadata tables; legacy body reservations remain recoverable until staging.
use super::super::*;
pub(crate) fn migrate(tx: &Transaction<'_>) -> Result<(), DbError> {
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS runner_candidate_stages (task_id TEXT NOT NULL REFERENCES tasks(task_id),attempt_id TEXT NOT NULL REFERENCES attempts(attempt_id),fence INTEGER NOT NULL,pointer_json TEXT NOT NULL,reason TEXT NOT NULL,PRIMARY KEY(attempt_id,pointer_json));
         CREATE TABLE IF NOT EXISTS board_handoffs (attempt_id TEXT PRIMARY KEY REFERENCES attempts(attempt_id),fence INTEGER NOT NULL,handoff_json TEXT NOT NULL);
         CREATE TABLE IF NOT EXISTS board_projection_state (path TEXT PRIMARY KEY,block_hash TEXT NOT NULL);
         CREATE TABLE IF NOT EXISTS runtime_audit_outbox (
           sequence INTEGER PRIMARY KEY AUTOINCREMENT,event_id TEXT NOT NULL UNIQUE,action TEXT NOT NULL,payload_json TEXT NOT NULL,delivered INTEGER NOT NULL DEFAULT 0);
         CREATE TABLE IF NOT EXISTS board_tasks (
           task_id TEXT PRIMARY KEY REFERENCES tasks(task_id), parent TEXT REFERENCES board_tasks(task_id),
           depth INTEGER NOT NULL CHECK(depth BETWEEN 0 AND 3), max_children INTEGER NOT NULL CHECK(max_children BETWEEN 0 AND 8),
           definition_json TEXT NOT NULL, handoff_json TEXT, receipt_json TEXT, created_by_instance TEXT);
         CREATE INDEX IF NOT EXISTS board_parent ON board_tasks(parent);
         CREATE TABLE IF NOT EXISTS board_requests (
           request_id TEXT PRIMARY KEY, request_json TEXT NOT NULL, task_id TEXT NOT NULL REFERENCES tasks(task_id));
         CREATE TABLE IF NOT EXISTS board_artifact_winners (
           project TEXT NOT NULL, target TEXT NOT NULL, task_id TEXT NOT NULL REFERENCES tasks(task_id),
           PRIMARY KEY(project, target));
         CREATE TABLE IF NOT EXISTS board_numbers (
           namespace TEXT NOT NULL, number INTEGER NOT NULL, task_id TEXT NOT NULL REFERENCES tasks(task_id),
           request_id TEXT NOT NULL UNIQUE, PRIMARY KEY(namespace, number));
         CREATE TABLE IF NOT EXISTS attempt_progress (
           attempt_id TEXT PRIMARY KEY REFERENCES attempts(attempt_id), tool_calls INTEGER NOT NULL DEFAULT 0,
           edits INTEGER NOT NULL DEFAULT 0, last_progress_ms INTEGER NOT NULL,
           cancel_requested_ms INTEGER, terminal_reason TEXT);
         INSERT OR IGNORE INTO attempt_progress(attempt_id,last_progress_ms,cancel_requested_ms)
           SELECT attempt_id,created_at_ms,CASE WHEN phase='cancel_requested' THEN updated_at_ms END FROM attempts;
         CREATE TABLE IF NOT EXISTS task_launch_retries (
           task_id TEXT PRIMARY KEY REFERENCES tasks(task_id), retries INTEGER NOT NULL,
           not_before_ms INTEGER NOT NULL);"
    )?;
    Ok(())
}
