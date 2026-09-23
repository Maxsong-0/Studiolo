//! `policy_current` 表：现行版本、校验上下文、现行能力集、现行记录与 effective 条目。

use std::collections::{BTreeMap, BTreeSet};

use rusqlite::{OptionalExtension as _, Transaction, params};

use crate::{
    PolicyError, PolicyKind,
    database::unavailable,
    model::CapabilitySet,
    store::{EffectiveEntry, VersionMeta, VersionRecord},
    validate::Context,
};

use super::{meta::effective_hash, version::version_record};

pub(crate) fn current(
    tx: &Transaction<'_>,
    kind: PolicyKind,
    subject: &str,
) -> Result<Option<(u64, String)>, PolicyError> {
    tx.query_row(
        "SELECT v.version, v.body FROM policy_current c
         JOIN policy_version v ON v.version = c.version
         WHERE c.kind = ?1 AND c.subject = ?2",
        params![kind.as_str(), subject],
        |row| {
            Ok((
                row.get::<_, i64>(0)?.max(0) as u64,
                row.get::<_, String>(1)?,
            ))
        },
    )
    .optional()
    .map_err(unavailable)
}

pub(crate) fn context(
    tx: &Transaction<'_>,
    kind: PolicyKind,
    subject: &str,
) -> Result<Context, PolicyError> {
    let mut skills = BTreeSet::new();
    let mut bindings: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut statement = tx
        .prepare(
            "SELECT c.kind, c.subject, v.body FROM policy_current c
             JOIN policy_version v ON v.version = c.version
             WHERE c.kind IN ('skills', 'capability_set')",
        )
        .map_err(unavailable)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(unavailable)?;
    for row in rows {
        let (row_kind, row_subject, body) = row.map_err(unavailable)?;
        if row_kind == "skills" {
            skills.insert(row_subject);
        } else if let Ok(set) = serde_json::from_str::<CapabilitySet>(&body) {
            for binding in set.hard_bound_skills {
                bindings
                    .entry(binding.must_use)
                    .or_default()
                    .push(row_subject.clone());
            }
        }
    }
    Ok(Context {
        current: current(tx, kind, subject)?,
        effective_hash: effective_hash(tx)?,
        skills,
        bindings,
    })
}

pub(crate) fn capability_set(
    tx: &Transaction<'_>,
    ai_type: &str,
) -> Result<Option<(CapabilitySet, VersionMeta)>, PolicyError> {
    let version: Option<i64> = tx
        .query_row(
            "SELECT version FROM policy_current WHERE kind = 'capability_set' AND subject = ?1",
            params![ai_type],
            |row| row.get(0),
        )
        .optional()
        .map_err(unavailable)?;
    let Some(version) = version else {
        return Ok(None);
    };
    let Some(record) = version_record(tx, version.max(0) as u64)? else {
        return Err(PolicyError::CorruptStore);
    };
    let body = record.body.as_deref().ok_or(PolicyError::CorruptStore)?;
    let set = serde_json::from_str::<CapabilitySet>(body).map_err(|_| PolicyError::CorruptStore)?;
    Ok(Some((set, record.meta)))
}

/// 现行（未撤销）版本，按类别过滤。
pub(crate) fn current_records(
    tx: &Transaction<'_>,
    kind: PolicyKind,
) -> Result<Vec<VersionRecord>, PolicyError> {
    let mut statement = tx
        .prepare("SELECT version FROM policy_current WHERE kind = ?1 ORDER BY subject")
        .map_err(unavailable)?;
    let versions = statement
        .query_map(params![kind.as_str()], |row| row.get::<_, i64>(0))
        .map_err(unavailable)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(unavailable)?;
    let mut records = Vec::with_capacity(versions.len());
    for version in versions {
        records.push(version_record(tx, version.max(0) as u64)?.ok_or(PolicyError::CorruptStore)?);
    }
    Ok(records)
}

pub(crate) fn current_entries(tx: &Transaction<'_>) -> Result<Vec<EffectiveEntry>, PolicyError> {
    let mut statement = tx
        .prepare(
            "SELECT c.kind, c.subject, c.version, v.body_sha256, v.activated_at
             FROM policy_current c JOIN policy_version v ON v.version = c.version
             ORDER BY c.kind, c.subject",
        )
        .map_err(unavailable)?;
    let rows = statement
        .query_map([], |row| {
            Ok(EffectiveEntry {
                kind: PolicyKind::parse(&row.get::<_, String>(0)?).unwrap_or(PolicyKind::Rules),
                subject: row.get(1)?,
                version: row.get::<_, i64>(2)?.max(0) as u64,
                body_sha256: row.get(3)?,
                activated_at: row.get(4)?,
            })
        })
        .map_err(unavailable)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(unavailable)
}
