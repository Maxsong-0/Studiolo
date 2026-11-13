//! 已读事实只追加既有审计；有界尾扫不足时返回未知，不伪造全量未读计数。
use super::paging::Snapshot;
use crate::changes::{SeenInput, actor, check, expired};
use crate::{CanonicalWorkbench, auth_http::ContentPrincipal, editing::Problem};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use studiolo_audit_ledger::{
    AuditEvent, AuditEventId, AuditFilter, AuditLedger, AuditPhase, AuditRecord, SanitizedPayload,
};
use studiolo_core::{VaultPath, VaultUuid};
use studiolo_vault_broker::VaultReader;
const ACTION: &str = "content.changes_seen";
#[derive(Clone, Default)]
pub(super) struct Watermark {
    pub complete: bool,
    sequence: u64,
    epoch: String,
    observation: u64,
}
impl Watermark {
    pub fn broker(&self, sequence: u64) -> Option<bool> {
        if sequence <= self.sequence {
            Some(false)
        } else {
            self.complete.then_some(true)
        }
    }
    pub fn observed(&self, epoch: &str, index: u64) -> Option<bool> {
        if self.epoch == epoch && index <= self.observation {
            Some(false)
        } else {
            self.complete.then_some(true)
        }
    }
}
#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Payload {
    schema: u32,
    actor: String,
    vault_uuid: VaultUuid,
    project: Option<VaultPath>,
    snapshot: String,
    sequence: u64,
    epoch: String,
    observation: u64,
    generation: u64,
}
fn payload(event: &AuditEvent) -> Result<Payload, ()> {
    if event.action() != ACTION {
        return Err(());
    }
    let p: Payload = serde_json::from_value(event.payload().as_value().clone()).map_err(|_| ())?;
    if p.schema != 1
        || !crate::changes::scope(p.project.as_ref())
        || !crate::changes::token(&p.snapshot)
        || !crate::changes::token(&p.epoch)
        || p.actor.len() != 64
        || !p
            .actor
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return Err(());
    }
    Ok(p)
}
fn rows(audit: &dyn AuditLedger, after: u64, limit: usize) -> Result<Vec<AuditRecord>, ()> {
    let rows = audit
        .query(&AuditFilter::default().after_offset(after).with_limit(limit))
        .map_err(|_| ())?;
    if rows.len() > limit {
        return Err(());
    }
    for (i, row) in rows.iter().enumerate() {
        if row.offset() != after.checked_add(i as u64 + 1).ok_or(())?
            || row.verify_integrity().is_err()
        {
            return Err(());
        }
        if i > 0 && rows[i - 1].record_hash() != row.previous_hash() {
            return Err(());
        }
    }
    Ok(rows)
}
fn tail(audit: &dyn AuditLedger) -> Result<u64, ()> {
    let (mut low, mut high) = (0u64, 1u64);
    while !rows(audit, high - 1, 1)?.is_empty() {
        low = high;
        high = high.checked_mul(2).ok_or(())?;
    }
    while high - low > 1 {
        let mid = low + (high - low) / 2;
        if rows(audit, mid - 1, 1)?.is_empty() {
            high = mid;
        } else {
            low = mid;
        }
    }
    Ok(low)
}
pub(super) fn load(audit: Option<&dyn AuditLedger>, s: &Snapshot) -> Watermark {
    audit
        .and_then(|a| load_inner(a, s).ok())
        .unwrap_or_default()
}
fn load_inner(audit: &dyn AuditLedger, s: &Snapshot) -> Result<Watermark, ()> {
    let end = tail(audit)?;
    let start = end.saturating_sub(8192);
    let mut mark = Watermark {
        complete: start == 0,
        epoch: s.epoch.clone(),
        ..Default::default()
    };
    let mut offset = start;
    while offset < end {
        let batch = rows(audit, offset, (end - offset).min(1024) as usize)?;
        if batch.is_empty() {
            return Err(());
        }
        for row in &batch {
            let event = row.event();
            if event.action() != ACTION || event.phase() != AuditPhase::Outcome {
                continue;
            }
            let p = payload(event)?;
            if p.actor != s.actor || p.vault_uuid != s.vault || p.project != s.project {
                continue;
            }
            // 只有相同负载的Intent和Outcome成对出现才是已读事实。
            let pair = audit
                .query(
                    &AuditFilter::default()
                        .with_event_id(event.event_id().clone())
                        .with_limit(3),
                )
                .map_err(|_| ())?;
            if pair.len() != 2
                || pair[0].event().phase() != AuditPhase::Intent
                || pair[0].event().event_id() != event.event_id()
                || pair[0].offset() >= pair[1].offset()
                || pair[1].event() != event
                || pair.iter().any(|r| r.verify_integrity().is_err())
                || payload(pair[0].event())? != p
            {
                return Err(());
            }
            mark.sequence = mark.sequence.max(p.sequence);
            if p.epoch == s.epoch {
                mark.observation = mark.observation.max(p.observation);
            }
        }
        offset = batch.last().ok_or(())?.offset();
    }
    Ok(mark)
}
fn unavailable() -> Problem {
    Problem::new(
        StatusCode::SERVICE_UNAVAILABLE,
        "seen_unavailable",
        "已读记录暂不可用，内容仍可浏览；请稍后重试。",
    )
}
fn conflict() -> Problem {
    Problem::new(
        StatusCode::CONFLICT,
        "seen_conflict",
        "此请求编号已用于另一份已读快照，请重新操作。",
    )
}
fn append(audit: &dyn AuditLedger, id: AuditEventId, payload: &Payload) -> Result<(), Problem> {
    for phase in [AuditPhase::Intent, AuditPhase::Outcome] {
        let data = serde_json::to_value(payload).map_err(|_| unavailable())?;
        let event = AuditEvent::new(
            id.clone(),
            phase,
            ACTION,
            SanitizedPayload::try_from(data).map_err(|_| unavailable())?,
        );
        let row = audit.append(event.clone()).map_err(|_| unavailable())?;
        if row.event() != &event || row.verify_integrity().is_err() {
            return Err(unavailable());
        }
    }
    Ok(())
}
pub(super) fn mark(
    owner: &CanonicalWorkbench,
    principal: &ContentPrincipal,
    input: SeenInput,
) -> Result<Value, Problem> {
    let guard = owner.core.owner.lock().map_err(|_| unavailable())?;
    let broker = guard.as_ref().ok_or_else(unavailable)?;
    check(owner, principal)?;
    if broker.vault_uuid() != input.vault_uuid {
        return Err(Problem::forbidden());
    }
    let audit = owner
        .core
        .content_audit
        .as_deref()
        .ok_or_else(unavailable)?;
    let who = actor(principal);
    let id = AuditEventId::parse(format!("changes-seen:{who}:{}", input.request_id))
        .map_err(|_| crate::changes::invalid())?;
    let prior = audit
        .query(
            &AuditFilter::default()
                .with_event_id(id.clone())
                .with_limit(3),
        )
        .map_err(|_| unavailable())?;
    let data = if let Some(first) = prior.first() {
        if prior.len() > 2
            || first.event().phase() != AuditPhase::Intent
            || prior
                .iter()
                .any(|r| r.verify_integrity().is_err() || r.event().event_id() != &id)
        {
            return Err(unavailable());
        }
        let data = payload(first.event()).map_err(|_| unavailable())?;
        if data.actor != who
            || data.vault_uuid != input.vault_uuid
            || data.project != input.project
            || data.snapshot != input.snapshot
        {
            return Err(conflict());
        }
        if prior.len() == 2
            && (prior[1].event().phase() != AuditPhase::Outcome
                || payload(prior[1].event()).map_err(|_| unavailable())? != data)
        {
            return Err(unavailable());
        }
        data
    } else {
        let snapshot = owner
            .core
            .change_pages
            .lock()
            .map_err(|_| unavailable())?
            .snapshot(&input.snapshot, &who, input.vault_uuid, &input.project)?;
        let log = owner.core.changes.lock().map_err(|_| unavailable())?;
        if snapshot.epoch != log.epoch || snapshot.observation > log.last_index {
            return Err(expired());
        }
        Payload {
            schema: 1,
            actor: who,
            vault_uuid: snapshot.vault,
            project: snapshot.project,
            snapshot: input.snapshot,
            sequence: snapshot.through,
            epoch: snapshot.epoch,
            observation: snapshot.observation,
            generation: snapshot.generation,
        }
    };
    check(owner, principal)?;
    append(audit, id, &data)?;
    check(owner, principal)?;
    Ok(
        json!({"status":"seen","vault_uuid":data.vault_uuid,"project":data.project,"through":{"sequence":data.sequence.to_string(),"epoch":data.epoch,"observation":data.observation.to_string(),"generation":data.generation.to_string()}}),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    use studiolo_audit_ledger::MemoryAuditLedger;
    fn snapshot() -> Snapshot {
        Snapshot {
            actor: "a".repeat(64),
            vault: VaultUuid::new(),
            project: None,
            through: 10,
            epoch: VaultUuid::new().to_string(),
            observation: 8,
            generation: 4,
        }
    }
    fn data(s: &Snapshot) -> Payload {
        Payload {
            schema: 1,
            actor: s.actor.clone(),
            vault_uuid: s.vault,
            project: s.project.clone(),
            snapshot: VaultUuid::new().to_string(),
            sequence: s.through,
            epoch: s.epoch.clone(),
            observation: s.observation,
            generation: s.generation,
        }
    }
    #[test]
    fn watermarks_are_audit_backed_scoped_monotonic_and_restart_safe() {
        let ledger = MemoryAuditLedger::new();
        let s = snapshot();
        let p = data(&s);
        let id = AuditEventId::parse("first").unwrap();
        assert_eq!(load(Some(&ledger), &s).broker(1), Some(true));
        append(&ledger, id.clone(), &p).unwrap_or_else(|_| panic!("append"));
        append(&ledger, id, &p).unwrap_or_else(|_| panic!("replay"));
        assert_eq!(ledger.query(&AuditFilter::default()).unwrap().len(), 2);
        let mut older = p.clone();
        older.sequence = 2;
        older.observation = 1;
        append(&ledger, AuditEventId::parse("older").unwrap(), &older)
            .unwrap_or_else(|_| panic!("older"));
        let m = load(Some(&ledger), &s);
        assert_eq!(m.broker(10), Some(false));
        assert_eq!(m.broker(11), Some(true));
        assert_eq!(m.observed(&s.epoch, 8), Some(false));
        assert_eq!(m.observed(&VaultUuid::new().to_string(), 1), Some(true));
        let mut project = s.clone();
        project.project = Some(VaultPath::parse("项目/Alpha").unwrap());
        assert_eq!(load(Some(&ledger), &project).broker(1), Some(true));
        let mut other = s;
        other.actor = "b".repeat(64);
        assert_eq!(load(Some(&ledger), &other).broker(1), Some(true));
        assert_eq!(load(None, &other).broker(1), None);
    }
    #[test]
    fn intent_alone_never_acknowledges_seen_and_a_broken_pair_is_unknown() {
        let ledger = MemoryAuditLedger::new();
        let s = snapshot();
        let p = data(&s);
        let id = AuditEventId::parse("pending").unwrap();
        let event = AuditEvent::new(
            id.clone(),
            AuditPhase::Intent,
            ACTION,
            SanitizedPayload::try_from(serde_json::to_value(&p).unwrap()).unwrap(),
        );
        ledger.append(event).unwrap();
        assert_eq!(load(Some(&ledger), &s).broker(1), Some(true));
        let mut wrong = p;
        wrong.sequence = 9;
        ledger
            .append(AuditEvent::new(
                id,
                AuditPhase::Outcome,
                ACTION,
                SanitizedPayload::try_from(serde_json::to_value(&wrong).unwrap()).unwrap(),
            ))
            .unwrap();
        assert_eq!(load(Some(&ledger), &s).broker(1), None);
    }
}
