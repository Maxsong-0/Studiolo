//! `policy_revocation` 表：生效中吊销的判定与吊销记录列表。

use rusqlite::{OptionalExtension as _, Transaction, params};

use crate::{
    PolicyError,
    database::unavailable,
    store::{RevocationLevel, RevocationRecord},
};

pub(crate) fn revoked(
    tx: &Transaction<'_>,
    level: RevocationLevel,
    target: &str,
) -> Result<bool, PolicyError> {
    let found: Option<i64> = tx
        .query_row(
            "SELECT 1 FROM policy_revocation WHERE level = ?1 AND target = ?2 AND released_at IS NULL",
            params![level.as_str(), target],
            |row| row.get(0),
        )
        .optional()
        .map_err(unavailable)?;
    Ok(found.is_some())
}

fn revocation_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RevocationRecord> {
    Ok(RevocationRecord {
        seq: row.get::<_, i64>(0)?.max(0) as u64,
        level: RevocationLevel::parse(&row.get::<_, String>(1)?).unwrap_or(RevocationLevel::Global),
        target: row.get(2)?,
        revoked_at: row.get(3)?,
        revoked_by: row.get(4)?,
        reason: row.get(5)?,
        released_at: row.get(6)?,
        released_by: row.get(7)?,
    })
}

pub(crate) fn revocations(
    tx: &Transaction<'_>,
    active_only: bool,
) -> Result<Vec<RevocationRecord>, PolicyError> {
    let mut statement = tx
        .prepare(
            "SELECT seq, level, target, revoked_at, revoked_by, reason, released_at, released_by
             FROM policy_revocation
             WHERE (?1 = 0 OR released_at IS NULL)
             ORDER BY seq DESC LIMIT 1000",
        )
        .map_err(unavailable)?;
    let rows = statement
        .query_map(params![i64::from(active_only)], revocation_from_row)
        .map_err(unavailable)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(unavailable)
}
