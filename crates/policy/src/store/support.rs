//! 内部辅助：skills 解析、effective 摘要、提案上限，以及发布用的哈希 CAS 与写版本。

use crate::{
    PolicyError, PolicyKind, Report,
    model::{Skill, sha256_hex},
    sql,
};

use super::{
    Activation, EffectiveSummary, GLOBAL_TARGET, MAX_PENDING_PER_INSTANCE, MAX_PENDING_PROPOSALS,
    RevocationLevel, VersionMeta, VersionRecord,
};

pub(crate) fn parse_skills(
    records: Vec<VersionRecord>,
) -> Result<Vec<(Skill, VersionMeta)>, PolicyError> {
    records
        .into_iter()
        .map(|record| {
            let body = record.body.ok_or(PolicyError::CorruptStore)?;
            let skill =
                serde_json::from_str::<Skill>(&body).map_err(|_| PolicyError::CorruptStore)?;
            Ok((skill, record.meta))
        })
        .collect()
}

pub(crate) fn effective_summary(
    tx: &rusqlite::Transaction<'_>,
) -> Result<EffectiveSummary, PolicyError> {
    let pending: i64 = tx
        .query_row(
            "SELECT count(*) FROM policy_desired WHERE status IN ('pending', 'validated')",
            [],
            |row| row.get(0),
        )
        .map_err(crate::database::unavailable)?;
    Ok(EffectiveSummary {
        effective_hash: sql::effective_hash(tx)?,
        entries: sql::current_entries(tx)?,
        global_revoked: sql::revoked(tx, RevocationLevel::Global, GLOBAL_TARGET)?,
        pending_desired: pending.max(0) as usize,
    })
}

pub(crate) fn enforce_proposal_bounds(
    tx: &rusqlite::Transaction<'_>,
    instance_id: &str,
) -> Result<(), PolicyError> {
    let per_instance: i64 = tx
        .query_row(
            "SELECT count(*) FROM policy_desired WHERE agent_instance = ?1 AND status = 'pending'",
            rusqlite::params![instance_id],
            |row| row.get(0),
        )
        .map_err(crate::database::unavailable)?;
    let total: i64 = tx
        .query_row(
            "SELECT count(*) FROM policy_desired WHERE submitter_kind = 'agent' AND status = 'pending'",
            [],
            |row| row.get(0),
        )
        .map_err(crate::database::unavailable)?;
    if per_instance >= MAX_PENDING_PER_INSTANCE as i64 || total >= MAX_PENDING_PROPOSALS as i64 {
        return Err(PolicyError::TooManyProposals);
    }
    Ok(())
}

pub(super) fn check_hash(
    tx: &rusqlite::Transaction<'_>,
    expected: &str,
) -> Result<(), PolicyError> {
    let current = sql::effective_hash(tx)?;
    if current != expected {
        return Err(PolicyError::HashMismatch { current });
    }
    Ok(())
}

pub(super) fn publish(
    tx: &rusqlite::Transaction<'_>,
    report: &Report,
    actor: &str,
    now: &str,
    desired_id: Option<&str>,
    rollback_of: Option<u64>,
) -> Result<Activation, PolicyError> {
    let previous = sql::effective_hash(tx)?;
    let body = report.canonical.clone().unwrap_or_default();
    let body_sha256 = if report.retire {
        sha256_hex(b"")
    } else {
        report.body_sha256.clone()
    };
    let version = sql::insert_version(
        tx,
        report.kind,
        &report.subject,
        &body,
        &body_sha256,
        report.retire,
        &previous,
        now,
        actor,
        desired_id,
        rollback_of,
        &report.expansions,
    )?;
    let effective_hash = sql::effective_hash(tx)?;
    let cleared_type_revocation = if report.kind == PolicyKind::CapabilitySet && !report.retire {
        tx.execute(
            "UPDATE policy_revocation SET released_at = ?1, released_by = ?2
             WHERE level = 'type' AND target = ?3 AND released_at IS NULL",
            rusqlite::params![now, actor, report.subject],
        )
        .map_err(crate::database::unavailable)?
            > 0
    } else {
        false
    };
    Ok(Activation {
        version,
        kind: report.kind,
        subject: report.subject.clone(),
        retired: report.retire,
        body_sha256,
        effective_hash,
        previous_effective_hash: previous,
        expansions: report.expansions.clone(),
        desired_id: desired_id.map(str::to_owned),
        rollback_of,
        cleared_type_revocation,
        activated_by: actor.to_owned(),
    })
}
