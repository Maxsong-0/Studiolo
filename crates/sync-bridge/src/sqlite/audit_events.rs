//! 固定事件构造；只记录摘要、计数、动作，不把 Input 放入审计。
use super::{BridgeError, RECEIPT_MAX, audit::Slot, codec::json, model::*, receipts};
use serde_json::json as value;
use studiolo_audit_ledger::{AuditEvent, AuditEventId, AuditPhase, SanitizedPayload};

pub(super) fn event(s: &State, slot: Slot) -> Result<AuditEvent> {
    let link = receipts::hex(&s.binding.link_id);
    let (id, phase, action, payload) = match slot {
        Slot::Intent(i) | Slot::Outcome(i) => {
            let p = s.pending.as_ref().ok_or(BridgeError::NotFound)?;
            action_event(p, i, matches!(slot, Slot::Outcome(_)))?
        }
        Slot::Summary => {
            let p = s.pending.as_ref().ok_or(BridgeError::NotFound)?;
            summary_parts(p)?
        }
        Slot::Fuse => {
            let f = s.fuse.as_ref().ok_or(BridgeError::NotFound)?;
            let crate::Decision::FuseBlocked(blocked) =
                crate::plan(f.input.clone(), None).map_err(|_| BridgeError::CorruptStore)?
            else {
                return Err(BridgeError::CorruptStore);
            };
            (
                format!("bridge:{link}:fuse:{}", f.input.plan_sequence),
                AuditPhase::Outcome,
                "bridge.fused",
                value!({"sequence":f.input.plan_sequence,"input_digest":receipts::hex(&receipts::input_hash(&f.input)?),
                    "canonical_deletes":blocked.stats.canonical,"mirror_deletes":blocked.stats.mirror,
                    "baseline_live_paths":blocked.stats.baseline_live_paths,"max_count":f.input.limits.deletion.max_count,
                    "basis_points":f.input.limits.deletion.max_basis_points}),
            )
        }
        Slot::Retirement => {
            let r = s.retirement.as_ref().ok_or(BridgeError::NotFound)?;
            (
                format!("bridge:{link}:retire:{}", r.old_sequence),
                AuditPhase::Outcome,
                "bridge.retirement_authorized",
                value!({"sequence":r.old_sequence,"previous_floor":r.old_floor,
                    "terminal_digest":receipts::hex(&r.old_digest),"retired_keys_rejected":true}),
            )
        }
    };
    make(id, phase, action, payload)
}

pub(super) fn action_event(
    p: &Pending,
    index: usize,
    outcome: bool,
) -> Result<(String, AuditPhase, &'static str, serde_json::Value)> {
    let a = p.actions.get(index).ok_or(BridgeError::NotFound)?;
    let result = if outcome {
        Some(receipts::hex(&receipts::hash(
            json(
                a.receipt.as_ref().ok_or(BridgeError::NotSettled)?,
                RECEIPT_MAX,
            )?
            .as_bytes(),
        )))
    } else {
        None
    };
    Ok((
        a.key.clone(),
        if outcome {
            AuditPhase::Outcome
        } else {
            AuditPhase::Intent
        },
        "bridge.action",
        value!({"plan_sequence":p.input.plan_sequence,"request_digest":receipts::hex(&receipts::request_hash(p,index)?),
            "outcome_digest":result}),
    ))
}
pub(super) fn summary_parts(
    p: &Pending,
) -> Result<(String, AuditPhase, &'static str, serde_json::Value)> {
    if !receipts::known(p) {
        return Err(BridgeError::NotSettled);
    }
    let mut bytes = Vec::new();
    for a in &p.actions {
        bytes.extend_from_slice(&receipts::hash(
            json(
                a.receipt.as_ref().ok_or(BridgeError::NotSettled)?,
                RECEIPT_MAX,
            )?
            .as_bytes(),
        ));
    }
    Ok((
        format!(
            "bridge:{}:settled:{}",
            receipts::hex(&p.input.scope.binding.link_id),
            p.input.plan_sequence
        ),
        AuditPhase::Outcome,
        "bridge.plan_results_known",
        value!({"plan_sequence":p.input.plan_sequence,
            "input_digest":receipts::hex(&receipts::input_hash(&p.input)?),"results_digest":receipts::hex(&receipts::hash(&bytes)),
            "all_committed":receipts::succeeded(p),"actions":p.actions.len()}),
    ))
}
pub(super) fn make(
    id: String,
    phase: AuditPhase,
    action: &str,
    payload: serde_json::Value,
) -> Result<AuditEvent> {
    let event = AuditEvent::new(
        AuditEventId::parse(id).map_err(|_| BridgeError::CorruptStore)?,
        phase,
        action,
        SanitizedPayload::try_from(payload).map_err(|_| BridgeError::CorruptStore)?,
    );
    json(&event, 1024 * 1024)?;
    Ok(event)
}
