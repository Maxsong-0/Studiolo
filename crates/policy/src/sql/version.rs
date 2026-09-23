//! `policy_version` 表：行映射、按号读取、版本列表与写新版本（同步 current 与哈希）。

use rusqlite::{OptionalExtension as _, Transaction, params};

use crate::{
    Expansion, PolicyError, PolicyKind,
    database::unavailable,
    store::{VersionMeta, VersionRecord},
};

use super::meta::recompute_effective;

fn meta_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<VersionMeta> {
    let expansions: String = row.get(10)?;
    Ok(VersionMeta {
        version: row.get::<_, i64>(0)?.max(0) as u64,
        kind: PolicyKind::parse(&row.get::<_, String>(1)?).unwrap_or(PolicyKind::Rules),
        subject: row.get(2)?,
        retired: row.get::<_, i64>(3)? == 1,
        body_sha256: row.get(4)?,
        effective_hash: row.get(5)?,
        previous_effective_hash: row.get(6)?,
        activated_at: row.get(7)?,
        activated_by: row.get(8)?,
        desired_id: row.get(9)?,
        rollback_of: row
            .get::<_, Option<i64>>(11)?
            .map(|value| value.max(0) as u64),
        expansions: serde_json::from_str::<Vec<Expansion>>(&expansions).unwrap_or_default(),
    })
}

const META_COLUMNS: &str = "version, kind, subject, retired, body_sha256, effective_hash, previous_effective_hash, activated_at, activated_by, desired_id, expansions, rollback_of";

pub(crate) fn version_record(
    tx: &Transaction<'_>,
    version: u64,
) -> Result<Option<VersionRecord>, PolicyError> {
    tx.query_row(
        &format!("SELECT {META_COLUMNS}, body FROM policy_version WHERE version = ?1"),
        params![version as i64],
        |row| {
            let meta = meta_from_row(row)?;
            let body: String = row.get(12)?;
            Ok(VersionRecord {
                body: (!meta.retired).then_some(body),
                meta,
            })
        },
    )
    .optional()
    .map_err(unavailable)
}

pub(crate) fn versions(
    tx: &Transaction<'_>,
    kind: Option<PolicyKind>,
    subject: Option<&str>,
    limit: usize,
) -> Result<Vec<VersionMeta>, PolicyError> {
    let mut statement = tx
        .prepare(&format!(
            "SELECT {META_COLUMNS} FROM policy_version
             WHERE (?1 IS NULL OR kind = ?1) AND (?2 IS NULL OR subject = ?2)
             ORDER BY version DESC LIMIT ?3"
        ))
        .map_err(unavailable)?;
    let rows = statement
        .query_map(
            params![
                kind.map(PolicyKind::as_str),
                subject,
                limit.clamp(1, 10_000) as i64
            ],
            meta_from_row,
        )
        .map_err(unavailable)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(unavailable)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn insert_version(
    tx: &Transaction<'_>,
    kind: PolicyKind,
    subject: &str,
    body: &str,
    body_sha256: &str,
    retired: bool,
    previous_effective_hash: &str,
    now: &str,
    actor: &str,
    desired_id: Option<&str>,
    rollback_of: Option<u64>,
    expansions: &[Expansion],
) -> Result<u64, PolicyError> {
    let next: i64 = tx
        .query_row(
            "SELECT COALESCE(MAX(version), 0) + 1 FROM policy_version",
            [],
            |row| row.get(0),
        )
        .map_err(unavailable)?;
    let expansions_json =
        serde_json::to_string(expansions).map_err(|_| PolicyError::StoreUnavailable)?;
    tx.execute(
        "INSERT INTO policy_version
            (version, kind, subject, body, body_sha256, retired, effective_hash,
             previous_effective_hash, activated_at, activated_by, desired_id, rollback_of, expansions)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            next,
            kind.as_str(),
            subject,
            body,
            body_sha256,
            i64::from(retired),
            previous_effective_hash,
            now,
            actor,
            desired_id,
            rollback_of.map(|value| value as i64),
            expansions_json
        ],
    )
    .map_err(unavailable)?;
    if retired {
        tx.execute(
            "DELETE FROM policy_current WHERE kind = ?1 AND subject = ?2",
            params![kind.as_str(), subject],
        )
        .map_err(unavailable)?;
    } else {
        tx.execute(
            "INSERT INTO policy_current (kind, subject, version) VALUES (?1, ?2, ?3)
             ON CONFLICT(kind, subject) DO UPDATE SET version = excluded.version",
            params![kind.as_str(), subject, next],
        )
        .map_err(unavailable)?;
    }
    let effective = recompute_effective(tx)?;
    tx.execute(
        "UPDATE policy_version SET effective_hash = ?2 WHERE version = ?1",
        params![next, effective],
    )
    .map_err(unavailable)?;
    Ok(next.max(0) as u64)
}
