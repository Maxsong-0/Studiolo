//! `policy_meta` 表：effective 哈希的初值、读取与由现行版本重算。

use rusqlite::{Transaction, params};

use crate::{PolicyError, database::unavailable, model::sha256_hex};

pub(crate) fn empty_effective_hash() -> String {
    sha256_hex(b"")
}

pub(crate) fn effective_hash(tx: &Transaction<'_>) -> Result<String, PolicyError> {
    tx.query_row(
        "SELECT value FROM policy_meta WHERE key = 'effective_hash'",
        [],
        |row| row.get(0),
    )
    .map_err(|_| PolicyError::CorruptStore)
}

/// 由现行版本重新计算 effective 哈希并写回 meta。
pub(crate) fn recompute_effective(tx: &Transaction<'_>) -> Result<String, PolicyError> {
    let mut statement = tx
        .prepare(
            "SELECT c.kind, c.subject, c.version, v.body_sha256
             FROM policy_current c JOIN policy_version v ON v.version = c.version
             ORDER BY c.kind, c.subject",
        )
        .map_err(unavailable)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(unavailable)?;
    let mut canonical = String::new();
    for row in rows {
        let (kind, subject, version, sha) = row.map_err(unavailable)?;
        canonical.push_str(&format!("{kind}\0{subject}\0{version}\0{sha}\n"));
    }
    let hash = sha256_hex(canonical.as_bytes());
    tx.execute(
        "UPDATE policy_meta SET value = ?1 WHERE key = 'effective_hash'",
        params![hash],
    )
    .map_err(unavailable)?;
    Ok(hash)
}
