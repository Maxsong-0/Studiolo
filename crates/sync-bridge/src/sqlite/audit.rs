//! 真实 append 在 SQLite 事务外；固定元数据事件，绝不把完整 Input 塞进审计。

pub(super) use super::audit_events::event;
use super::audit_events::{action_event, make, summary_parts};
use super::{BridgeError, BridgeState, RECEIPT_MAX, codec::json, model::*, receipts, state_codec};
use studiolo_audit_ledger::{AuditEvent, AuditLedger, AuditRecord};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Slot {
    Intent(usize),
    Outcome(usize),
    Summary,
    Retirement,
    Fuse,
}

impl BridgeState {
    /// 预览当前下一条固定事件，不产生 ACK。
    pub fn pending_audit(&self) -> Result<Option<AuditEvent>> {
        let state = self.read()?;
        next(state)?.map(|slot| event(state, slot)).transpose()
    }
    /// 调用方只能注入受信任的权威 adapter；普通 JSON/布尔值不是此接口。
    pub fn flush_audit(
        &mut self,
        ledger: &dyn AuditLedger,
        max_records: usize,
    ) -> Result<AuditDelivery> {
        if max_records == 0 || max_records > 64 {
            return Err(BridgeError::Budget);
        }
        let mut acknowledged = 0;
        while acknowledged < max_records {
            let state = self.read()?;
            let Some(slot) = next(state)? else {
                break;
            };
            let expected = event(state, slot)?;
            let record = ledger
                .append(expected.clone())
                .map_err(|_| BridgeError::AuditUnavailable)?;
            let encoded = json(&record, RECEIPT_MAX)?;
            check_ack(&encoded, &expected)?;
            // append 已返回。现在才开始 SQLite 事务；若失败，原事件下次原键追加。
            let edit = match slot {
                Slot::Intent(index) | Slot::Outcome(index) => Edit::Action {
                    sequence: state
                        .pending
                        .as_ref()
                        .ok_or(BridgeError::NotFound)?
                        .input
                        .plan_sequence,
                    index,
                },
                Slot::Summary => Edit::Summary,
                Slot::Retirement => Edit::Retirement,
                Slot::Fuse => Edit::FuseAck,
            };
            self.edit(edit, Some(state.version), |state| {
                if event(state, slot)? != expected {
                    return Err(BridgeError::VersionConflict);
                }
                *ack_mut(state, slot)? = Some(encoded);
                Ok(())
            })?;
            acknowledged += 1;
        }
        Ok(AuditDelivery {
            acknowledged,
            remaining: next(self.read()?)?.is_some(),
        })
    }
}

pub(super) fn next(s: &State) -> Result<Option<Slot>> {
    if s.fuse.as_ref().is_some_and(|f| f.ack.is_none()) {
        return Ok(Some(Slot::Fuse));
    }
    if let Some(p) = &s.pending {
        match (
            p.intent_waiting.first().copied(),
            p.outcome_waiting.first().copied(),
        ) {
            (Some(i), Some(o)) if o < i => return Ok(Some(Slot::Outcome(o))),
            (Some(i), _) => return Ok(Some(Slot::Intent(i))),
            (None, Some(o)) => return Ok(Some(Slot::Outcome(o))),
            _ => {}
        }
        if receipts::known(p) && p.outcome_waiting.is_empty() && p.summary_ack.is_none() {
            return Ok(Some(Slot::Summary));
        }
    }
    if s.retirement.as_ref().is_some_and(|r| r.ack.is_none()) {
        return Ok(Some(Slot::Retirement));
    }
    Ok(None)
}

pub(super) fn check_action(p: &Pending, i: usize) -> Result<()> {
    let a = &p.actions[i];
    for (ack, outcome) in [(&a.intent_ack, false), (&a.outcome_ack, true)] {
        if let Some(ack) = ack {
            let (id, phase, action, payload) = action_event(p, i, outcome)?;
            check_ack(ack, &make(id, phase, action, payload)?)?;
        }
    }
    Ok(())
}
pub(super) fn check_summary(p: &Pending) -> Result<()> {
    if let Some(ack) = &p.summary_ack {
        let (id, phase, action, payload) = summary_parts(p)?;
        check_ack(ack, &make(id, phase, action, payload)?)?;
    }
    Ok(())
}
pub(super) fn check_ack(encoded: &str, event: &AuditEvent) -> Result<()> {
    if encoded.len() > RECEIPT_MAX {
        return Err(BridgeError::Budget);
    }
    let record: AuditRecord =
        serde_json::from_str(encoded).map_err(|_| BridgeError::CorruptStore)?;
    if record.event() != event || json(&record, RECEIPT_MAX)? != encoded {
        return Err(BridgeError::CorruptStore);
    }
    record
        .verify_integrity()
        .map_err(|_| BridgeError::CorruptStore)
}
fn ack_mut(s: &mut State, slot: Slot) -> Result<&mut Option<String>> {
    Ok(match slot {
        Slot::Intent(i) => {
            &mut s
                .pending
                .as_mut()
                .ok_or(BridgeError::NotFound)?
                .actions
                .get_mut(i)
                .ok_or(BridgeError::NotFound)?
                .intent_ack
        }
        Slot::Outcome(i) => {
            &mut s
                .pending
                .as_mut()
                .ok_or(BridgeError::NotFound)?
                .actions
                .get_mut(i)
                .ok_or(BridgeError::NotFound)?
                .outcome_ack
        }
        Slot::Summary => &mut s.pending.as_mut().ok_or(BridgeError::NotFound)?.summary_ack,
        Slot::Fuse => &mut s.fuse.as_mut().ok_or(BridgeError::NotFound)?.ack,
        Slot::Retirement => &mut s.retirement.as_mut().ok_or(BridgeError::NotFound)?.ack,
    })
}
pub(crate) fn terminal_digest(p: &Pending) -> Result<crate::Digest> {
    Ok(receipts::hash(&state_codec::pending_bytes(p)?))
}
