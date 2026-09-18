//! 两个句柄共用的 SQL 片段：当前版本、校验上下文、effective 哈希、吊销状态、行映射。
//!
//! 所有函数都在调用方给定的事务内执行，不自己开连接。

use std::collections::{BTreeMap, BTreeSet};

use rusqlite::{OptionalExtension as _, Transaction, params};

use crate::{
    Expansion, PolicyError, PolicyKind, Report,
    database::unavailable,
    model::{CapabilitySet, sha256_hex},
    store::{
        DesiredRecord, DesiredStatus, EffectiveEntry, RevocationLevel, RevocationRecord,
        VersionMeta, VersionRecord,
    },
    validate::Context,
};

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

fn desired_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<DesiredRecord> {
    let report: String = row.get(10)?;
    Ok(DesiredRecord {
        id: row.get(0)?,
        kind: PolicyKind::parse(&row.get::<_, String>(1)?).unwrap_or(PolicyKind::Rules),
        subject: row.get(2)?,
        body: row.get(3)?,
        body_sha256: row.get(4)?,
        submitter_kind: row.get(5)?,
        agent_type: row.get(6)?,
        agent_instance: row.get(7)?,
        submitted_at: row.get(8)?,
        status: DesiredStatus::parse(&row.get::<_, String>(9)?).unwrap_or(DesiredStatus::Rejected),
        report: serde_json::from_str::<Report>(&report).ok(),
        validated_at: row.get(11)?,
        activated_version: row
            .get::<_, Option<i64>>(12)?
            .map(|value| value.max(0) as u64),
    })
}

const DESIRED_COLUMNS: &str = "id, kind, subject, body, body_sha256, submitter_kind, agent_type, agent_instance, submitted_at, status, report, validated_at, activated_version";

pub(crate) fn desired(
    tx: &Transaction<'_>,
    id: &str,
) -> Result<Option<DesiredRecord>, PolicyError> {
    tx.query_row(
        &format!("SELECT {DESIRED_COLUMNS} FROM policy_desired WHERE id = ?1"),
        params![id],
        desired_from_row,
    )
    .optional()
    .map_err(unavailable)
}

pub(crate) fn desired_list(
    tx: &Transaction<'_>,
    status: Option<DesiredStatus>,
    limit: usize,
) -> Result<Vec<DesiredRecord>, PolicyError> {
    let mut statement = tx
        .prepare(&format!(
            "SELECT {DESIRED_COLUMNS} FROM policy_desired
             WHERE (?1 IS NULL OR status = ?1)
             ORDER BY submitted_at DESC, id DESC LIMIT ?2"
        ))
        .map_err(unavailable)?;
    let rows = statement
        .query_map(
            params![
                status.map(DesiredStatus::as_str),
                limit.clamp(1, 10_000) as i64
            ],
            desired_from_row,
        )
        .map_err(unavailable)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(unavailable)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn insert_desired(
    tx: &Transaction<'_>,
    id: &str,
    report: &Report,
    body: Option<&str>,
    submitter_kind: &str,
    agent_type: Option<&str>,
    agent_instance: Option<&str>,
    now: &str,
    status: DesiredStatus,
) -> Result<(), PolicyError> {
    let report_json = serde_json::to_string(report).map_err(|_| PolicyError::StoreUnavailable)?;
    tx.execute(
        "INSERT INTO policy_desired
            (id, kind, subject, body, body_sha256, submitter_kind, agent_type, agent_instance,
             submitted_at, status, report, validated_at, activated_version)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, NULL)",
        params![
            id,
            report.kind.as_str(),
            report.subject,
            body,
            report.body_sha256,
            submitter_kind,
            agent_type,
            agent_instance,
            now,
            status.as_str(),
            report_json,
            now
        ],
    )
    .map_err(unavailable)?;
    Ok(())
}

pub(crate) fn update_desired(
    tx: &Transaction<'_>,
    id: &str,
    status: DesiredStatus,
    report: Option<&Report>,
    now: &str,
    activated_version: Option<u64>,
) -> Result<(), PolicyError> {
    let report_json = report
        .map(|report| serde_json::to_string(report).map_err(|_| PolicyError::StoreUnavailable))
        .transpose()?;
    tx.execute(
        "UPDATE policy_desired
         SET status = ?2,
             report = COALESCE(?3, report),
             validated_at = CASE WHEN ?3 IS NULL THEN validated_at ELSE ?4 END,
             activated_version = COALESCE(?5, activated_version)
         WHERE id = ?1",
        params![
            id,
            status.as_str(),
            report_json,
            now,
            activated_version.map(|value| value as i64)
        ],
    )
    .map_err(unavailable)?;
    Ok(())
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
