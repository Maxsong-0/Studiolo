//! v2 回收附加格式；只保留一个不可变计划和一个替换式检查点。

use super::{SessionError, database::unavailable, retention};
use rusqlite::Connection;

const CHECKPOINT: &str = "CREATE TABLE auth_retention_checkpoint (
 id INTEGER PRIMARY KEY CHECK(id=1), checkpoint_json TEXT NOT NULL, receipt_json TEXT
) STRICT";
const PLAN: &str = "CREATE TABLE auth_retention_plan (
 id INTEGER PRIMARY KEY CHECK(id=1), manifest_hash TEXT NOT NULL, plan_json TEXT NOT NULL
) STRICT";

pub(super) const OBJECTS: &[(&str, Option<&str>)] = &[
    ("auth_retention_checkpoint", Some(CHECKPOINT)),
    ("auth_retention_plan", Some(PLAN)),
];

pub(super) fn initialize(db: &Connection) -> Result<(), SessionError> {
    db.execute_batch(&format!("{CHECKPOINT};{PLAN};"))
        .map_err(unavailable)?;
    db.execute(
        "INSERT INTO auth_retention_checkpoint VALUES(1,?1,NULL)",
        [retention::genesis()?],
    )
    .map_err(unavailable)?;
    Ok(())
}

pub(super) fn validate(db: &Connection) -> Result<(), SessionError> {
    let (checkpoints, plans): (i64, i64) = db
        .query_row(
            "SELECT (SELECT count(*) FROM auth_retention_checkpoint),(SELECT count(*) FROM auth_retention_plan)",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(unavailable)?;
    if checkpoints != 1 || plans > 1 {
        return Err(SessionError::CorruptStore);
    }
    retention::validate(db)
}
