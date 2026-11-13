//! 检查当前版本、封闭对象集合、业务引用与真实排空；不执行迁移或补记。
use crate::{migration::SCHEMA_VERSION, types::parse_vault_uuid};
use rusqlite::Connection;
use studiolo_checkpoint_storage::Error;
use studiolo_core::{VaultUuid, checkpoint::OwnerWatermark};

pub(super) fn snapshot(db: &Connection, vault: VaultUuid) -> Result<OwnerWatermark, Error> {
    catalog(db)?;
    let (count, minimum, maximum): (i64, i64, i64) = db
        .query_row(
            "SELECT count(*),min(version),max(version) FROM schema_migrations",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map_err(invalid)?;
    if count != SCHEMA_VERSION || minimum != 1 || maximum != SCHEMA_VERSION {
        return Err(Error::Invalid);
    }
    let stored: String = db
        .query_row(
            "SELECT vault_uuid FROM vault_binding WHERE singleton=1",
            [],
            |r| r.get(0),
        )
        .map_err(invalid)?;
    if parse_vault_uuid(&stored).map_err(invalid)? != vault {
        return Err(Error::Invalid);
    }
    for query in [
        "SELECT count(*) FROM vault_binding WHERE singleton!=1",
        "SELECT count(*) FROM snapshots WHERE vault_uuid!=(SELECT vault_uuid FROM vault_binding WHERE singleton=1)",
        "SELECT count(*) FROM projection_outbox WHERE delivered_at_ms IS NULL OR sequence<=0 OR delivered_at_ms<0",
        "SELECT count(*) FROM terminal_audit_outbox WHERE delivered_at_ms IS NULL OR delivered_at_ms<0",
        "SELECT count(*) FROM runtime_audit_outbox WHERE delivered!=1 OR sequence<=0",
        "SELECT count(*) FROM attempts WHERE phase='committing'",
        "SELECT count(*) FROM attempts WHERE phase NOT IN ('claimed','running','cancel_requested','committing','completed','failed','cancelled','abandoned') OR fence<=0 OR updated_at_ms<created_at_ms",
        "SELECT count(*) FROM tasks WHERE status NOT IN ('draft','queued','claimed','running','committing','cancel_requested','needs_reconfirm','needs_review','needs_revision','approved','succeeded','failed','cancelled','abandoned') OR updated_at_ms<created_at_ms",
        "SELECT count(*) FROM tasks t WHERE active_attempt IS NOT NULL AND NOT EXISTS(SELECT 1 FROM attempts a WHERE a.attempt_id=t.active_attempt AND a.task_id=t.task_id AND a.phase IN ('claimed','running','cancel_requested'))",
        "SELECT count(*) FROM attempts a WHERE a.phase IN ('claimed','running','cancel_requested') AND NOT EXISTS(SELECT 1 FROM tasks t WHERE t.task_id=a.task_id AND t.active_attempt=a.attempt_id)",
        "SELECT count(*) FROM tasks t WHERE last_attempt IS NOT NULL AND NOT EXISTS(SELECT 1 FROM attempts a WHERE a.attempt_id=t.last_attempt AND a.task_id=t.task_id)",
        "SELECT count(*) FROM artifact_commits c WHERE NOT EXISTS(SELECT 1 FROM terminal_audit_outbox o WHERE o.attempt_id=c.attempt_id)",
        "SELECT count(*) FROM attempts WHERE terminal_json IS NOT NULL AND (NOT json_valid(terminal_json) OR json_type(terminal_json,'$.artifacts_json') IS NOT 'object' OR json_extract(terminal_json,'$.report_json.format') IS NOT 3)",
    ] {
        if scalar(db, query)? != 0 {
            return Err(Error::Invalid);
        }
    }
    if scalar(
        db,
        "SELECT count(*) FROM fence_counter WHERE singleton=1 AND next_fence>COALESCE((SELECT max(fence) FROM attempts),0)",
    )? != 1
    {
        return Err(Error::Invalid);
    }
    let check: String = db
        .query_row("PRAGMA quick_check(1)", [], |r| r.get(0))
        .map_err(invalid)?;
    if check != "ok" {
        return Err(Error::Invalid);
    }
    let mut fk = db.prepare("PRAGMA foreign_key_check").map_err(invalid)?;
    if fk
        .query([])
        .map_err(invalid)?
        .next()
        .map_err(invalid)?
        .is_some()
    {
        return Err(Error::Invalid);
    }
    let delivered = scalar(
        db,
        "SELECT COALESCE(max(sequence),0) FROM projection_outbox",
    )?;
    Ok(OwnerWatermark::Control {
        schema_version: SCHEMA_VERSION as u32,
        vault_uuid: vault,
        outbox_pending: 0,
        outbox_delivered_through: u64::try_from(delivered).map_err(invalid)?,
    })
}
fn catalog(db: &Connection) -> Result<(), Error> {
    const TABLES: &[&str] = &[
        "schema_migrations",
        "vault_binding",
        "fence_counter",
        "tasks",
        "snapshots",
        "snapshot_entries",
        "attempts",
        "processed_events",
        "projection_outbox",
        "artifact_commits",
        "terminal_audit_outbox",
        "runner_candidate_stages",
        "board_handoffs",
        "board_projection_state",
        "runtime_audit_outbox",
        "board_tasks",
        "board_requests",
        "board_artifact_winners",
        "board_numbers",
        "attempt_progress",
        "task_launch_retries",
        "sqlite_sequence",
    ];
    let mut tables = 0;
    let mut statement = db.prepare("SELECT type,CASE WHEN octet_length(name)<=128 THEN name END,CASE WHEN octet_length(tbl_name)<=128 THEN tbl_name END,sql IS NULL FROM sqlite_schema").map_err(invalid)?;
    let mut rows = statement.query([]).map_err(invalid)?;
    while let Some(row) = rows.next().map_err(invalid)? {
        let kind: String = row.get(0).map_err(invalid)?;
        let name: String = row.get(1).map_err(invalid)?;
        let table: String = row.get(2).map_err(invalid)?;
        let implicit: bool = row.get(3).map_err(invalid)?;
        match kind.as_str() {
            "table" if TABLES.contains(&name.as_str()) && name == table => tables += 1,
            "index"
                if TABLES.contains(&table.as_str())
                    && ((implicit && name.starts_with("sqlite_autoindex_"))
                        || matches!(
                            name.as_str(),
                            "one_active_attempt_per_task"
                                | "pending_terminal_audits"
                                | "board_parent"
                        )) => {}
            _ => return Err(Error::Invalid),
        }
    }
    if tables != TABLES.len() {
        return Err(Error::Invalid);
    }
    Ok(())
}
fn scalar(db: &Connection, query: &str) -> Result<i64, Error> {
    db.query_row(query, [], |r| r.get(0)).map_err(invalid)
}
fn invalid<T>(_: T) -> Error {
    Error::Invalid
}
