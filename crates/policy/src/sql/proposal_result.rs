//! 提案结果回流：先核对类型与当前规则名单，再读取最小结果字段。

use rusqlite::{OptionalExtension as _, Transaction, params};

use crate::{
    DesiredStatus, PolicyError, PolicyKind, ProposalResult, ProposalSummary, Report,
    database::unavailable,
};

pub(crate) fn proposal_result(
    tx: &Transaction<'_>,
    agent_type: &str,
    id: &str,
    rules_whitelist: &[String],
) -> Result<Option<ProposalResult>, PolicyError> {
    let identity: Option<(String, String)> = tx
        .query_row(
            "SELECT kind, subject FROM policy_desired
         WHERE id = ?1 AND submitter_kind = 'agent' AND agent_type = ?2",
            params![id, agent_type],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(unavailable)?;
    let Some((kind, subject)) = identity else {
        return Ok(None);
    };
    let kind = PolicyKind::parse(&kind).map_err(|_| PolicyError::CorruptStore)?;
    if kind == PolicyKind::Rules && !rules_whitelist.contains(&subject) {
        return Ok(None);
    }
    let (status, body_sha256, submitted_at, instance, version, report): (
        String,
        String,
        String,
        Option<String>,
        Option<i64>,
        String,
    ) = tx
        .query_row(
            "SELECT status, body_sha256, submitted_at, agent_instance, activated_version, report
         FROM policy_desired WHERE id = ?1",
            params![id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )
        .map_err(unavailable)?;
    let status = DesiredStatus::parse(&status).ok_or(PolicyError::CorruptStore)?;
    let version = version
        .map(u64::try_from)
        .transpose()
        .map_err(|_| PolicyError::CorruptStore)?;
    let problems = if status == DesiredStatus::Rejected {
        serde_json::from_str::<Report>(&report)
            .map_err(|_| PolicyError::CorruptStore)?
            .problems
    } else {
        Vec::new()
    };
    let reason = match status {
        DesiredStatus::Pending => "awaiting_review",
        DesiredStatus::Validated => "ready_to_publish",
        DesiredStatus::Rejected => "validation_failed",
        DesiredStatus::Activated => "published",
        DesiredStatus::Superseded => "superseded_by_publication",
        DesiredStatus::Withdrawn => "owner_withdrawn_no_reason_recorded",
    };
    Ok(Some(ProposalResult {
        proposal: ProposalSummary {
            id: id.to_owned(),
            kind,
            subject,
            status,
            body_sha256,
            submitted_at,
            agent_instance: instance,
        },
        activated_version: version,
        problems,
        reason,
    }))
}
