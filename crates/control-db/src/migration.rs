//! schema 版本号与幂等迁移，全部步骤在同一写事务内完成。
use super::*;

pub(super) const SCHEMA_VERSION: i64 = 3;

impl ControlDb {
    pub(super) fn migrate(&mut self) -> Result<(), DbError> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        // 在写事务取得后再次核对，不能以事务外的旧观察代替迁移准入。
        schema_guard::require_supported_version(&tx)?;
        tx.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_migrations (
                 version INTEGER PRIMARY KEY, applied_at_ms INTEGER NOT NULL);",
        )?;
        tx.execute_batch(
            "CREATE TABLE IF NOT EXISTS vault_binding (
                 singleton INTEGER PRIMARY KEY CHECK (singleton = 1), vault_uuid TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS fence_counter (
                 singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
                 next_fence INTEGER NOT NULL CHECK (next_fence > 0));
             INSERT OR IGNORE INTO fence_counter(singleton, next_fence) VALUES (1, 1);
             CREATE TABLE IF NOT EXISTS tasks (
                 task_id TEXT PRIMARY KEY, definition_json TEXT NOT NULL, status TEXT NOT NULL,
                 active_attempt TEXT, last_attempt TEXT, input_snapshot TEXT,
                 created_at_ms INTEGER NOT NULL, updated_at_ms INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS snapshots (
                 snapshot_id TEXT PRIMARY KEY, vault_uuid TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS snapshot_entries (
                 snapshot_id TEXT NOT NULL REFERENCES snapshots(snapshot_id) ON DELETE CASCADE,
                 path TEXT NOT NULL, revision TEXT NOT NULL, effective_label_json TEXT NOT NULL,
                 PRIMARY KEY(snapshot_id, path));
             CREATE TABLE IF NOT EXISTS attempts (
                 attempt_id TEXT PRIMARY KEY, task_id TEXT NOT NULL REFERENCES tasks(task_id),
                 fence INTEGER NOT NULL UNIQUE CHECK (fence > 0), phase TEXT NOT NULL,
                 worker_id TEXT NOT NULL, snapshot_id TEXT NOT NULL REFERENCES snapshots(snapshot_id),
                 lease_until_ms INTEGER NOT NULL, runner_ref_json TEXT, terminal_json TEXT,
                 created_at_ms INTEGER NOT NULL, updated_at_ms INTEGER NOT NULL);
             CREATE UNIQUE INDEX IF NOT EXISTS one_active_attempt_per_task ON attempts(task_id)
                 WHERE phase IN ('claimed', 'running', 'cancel_requested', 'committing');
             CREATE TABLE IF NOT EXISTS processed_events (
                 event_id TEXT PRIMARY KEY, result_json TEXT NOT NULL, processed_at_ms INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS projection_outbox (
                 sequence INTEGER PRIMARY KEY AUTOINCREMENT, event_key TEXT NOT NULL UNIQUE,
                 task_id TEXT NOT NULL REFERENCES tasks(task_id), payload_json TEXT NOT NULL,
                 delivered_at_ms INTEGER);
             CREATE TABLE IF NOT EXISTS artifact_commits (
                 attempt_id TEXT PRIMARY KEY REFERENCES attempts(attempt_id),
                 receipt_json TEXT NOT NULL, committed_at_ms INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS terminal_audit_outbox (
                 attempt_id TEXT PRIMARY KEY REFERENCES artifact_commits(attempt_id),
                 status TEXT NOT NULL, delivered_at_ms INTEGER);
             CREATE INDEX IF NOT EXISTS pending_terminal_audits
                 ON terminal_audit_outbox(attempt_id) WHERE delivered_at_ms IS NULL;",
        )?;
        let has_input_snapshot = {
            let mut columns = tx.prepare("PRAGMA table_info(tasks)")?;
            let names = columns
                .query_map([], |row| row.get::<_, String>(1))?
                .collect::<Result<Vec<_>, _>>()?;
            names.iter().any(|name| name == "input_snapshot")
        };
        if !has_input_snapshot {
            tx.execute_batch(
                "ALTER TABLE tasks ADD COLUMN input_snapshot TEXT;
                UPDATE tasks SET input_snapshot = (
                    SELECT snapshot_id FROM attempts
                    WHERE attempt_id = COALESCE(tasks.active_attempt, tasks.last_attempt)
                );",
            )?;
        }
        tx.execute(
            "INSERT OR IGNORE INTO schema_migrations(version, applied_at_ms) VALUES (1, 0)",
            [],
        )?;
        let previous_version: i64 =
            tx.query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
                row.get(0)
            })?;
        if previous_version < 2 {
            // v1 在确认审计已持久之前就完成了终结。这次一次性迁移修复是安全的，
            // 因为账本会按原 event/phase 去重。
            tx.execute(
                "INSERT OR IGNORE INTO terminal_audit_outbox(attempt_id, status)
                 SELECT a.attempt_id, t.status FROM attempts a
                 JOIN artifact_commits c USING(attempt_id) JOIN tasks t USING(task_id)
                 WHERE a.phase IN ('completed', 'failed', 'cancelled') AND a.terminal_json IS NOT NULL",
                [],
            )?;
        }
        board::migrate(&tx)?;
        tx.execute(
            "INSERT OR IGNORE INTO schema_migrations(version, applied_at_ms) VALUES (2, 0)",
            [],
        )?;
        tx.execute(
            "INSERT OR IGNORE INTO schema_migrations(version, applied_at_ms) VALUES (?1, 0)",
            [SCHEMA_VERSION],
        )?;
        tx.commit()?;
        Ok(())
    }
}
