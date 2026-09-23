//! `policy_desired` 表：行映射、按 id 读取、列表、插入与状态更新。

use rusqlite::{OptionalExtension as _, Transaction, params};

use crate::{
    PolicyError, PolicyKind, Report,
    database::unavailable,
    store::{DesiredRecord, DesiredStatus},
};

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
