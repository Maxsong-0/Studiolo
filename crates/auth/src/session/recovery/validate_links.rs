//! 固定码批、已用位图、消费锚点与重复决定之间的完整关联。

use super::check;
use crate::session::{
    SessionError,
    recovery::model::{Code, Control, Kind, Operation, Pending},
    totp::model::Factor,
};
use std::collections::BTreeMap;

type Acknowledgments = (Option<String>, Option<String>);

struct Seen {
    operation: Operation,
    receipts: Option<Acknowledgments>,
}

#[derive(Default)]
pub(super) struct Registry {
    operations: BTreeMap<i64, Seen>,
    targets: BTreeMap<i64, i64>,
}

impl Registry {
    pub fn add(
        &mut self,
        operation: &Operation,
        receipts: Option<(Option<&str>, Option<&str>)>,
    ) -> Result<(), SessionError> {
        let receipts = receipts
            .map(|(intent, outcome)| (intent.map(str::to_owned), outcome.map(str::to_owned)));
        if let Some(seen) = self.operations.get_mut(&operation.sequence) {
            if seen.operation != *operation {
                return Err(SessionError::CorruptStore);
            }
            match (&seen.receipts, &receipts) {
                (Some(before), Some(after)) if before != after => {
                    return Err(SessionError::CorruptStore);
                }
                (None, Some(_)) => seen.receipts = receipts,
                _ => {}
            }
        } else {
            if let Some(target) = &operation.target
                && self
                    .targets
                    .insert(target.sequence, operation.sequence)
                    .is_some()
            {
                return Err(SessionError::CorruptStore);
            }
            self.operations.insert(
                operation.sequence,
                Seen {
                    operation: operation.clone(),
                    receipts,
                },
            );
        }
        Ok(())
    }

    pub fn pending(&mut self, pending: &Pending) -> Result<(), SessionError> {
        self.add(
            &pending.operation,
            Some((
                pending.intent_receipt.as_deref(),
                pending.outcome_receipt.as_deref(),
            )),
        )
    }

    pub fn maximum(&self) -> i64 {
        self.operations
            .last_key_value()
            .map_or(0, |(sequence, _)| *sequence)
    }

    pub fn consumption_order(&self) -> Result<(), SessionError> {
        let mut previous = None;
        for seen in self.operations.values() {
            let Some(target) = &seen.operation.target else {
                continue;
            };
            if previous
                .is_some_and(|(sequence, step)| target.sequence <= sequence || target.step <= step)
            {
                return Err(SessionError::CorruptStore);
            }
            previous = Some((target.sequence, target.step));
        }
        Ok(())
    }
}

pub(super) fn covered(operation: &Operation, control: &Control) -> Result<(), SessionError> {
    let anchor = control
        .consumption
        .as_ref()
        .ok_or(SessionError::CorruptStore)?;
    let target = operation
        .target
        .as_ref()
        .ok_or(SessionError::CorruptStore)?;
    if operation.kind != Kind::Replace
        || operation.sequence > anchor.sequence
        || target.step > control.high_step
        || (operation.sequence == anchor.sequence && operation != anchor)
    {
        return Err(SessionError::CorruptStore);
    }
    Ok(())
}

pub(super) fn codes(
    codes: &[Code],
    control: &Control,
    pending: Option<&Pending>,
    factor: &Factor,
    bounds: &check::Bounds,
    registry: &mut Registry,
) -> Result<(), SessionError> {
    let staged = pending.filter(|p| p.operation.kind == Kind::Rotate);
    let current = control.current.as_ref();
    let mut current_count = 0;
    let mut staged_count = 0;
    let mut used_mask = 0_u16;
    let mut matched_pending = false;
    for code in codes {
        if let Some(current) = current.filter(|p| p.operation.generation == code.generation) {
            current_count += 1;
            if current.operation.hashes.get(usize::from(code.slot)) != Some(&code.digest) {
                return Err(SessionError::CorruptStore);
            }
            let Some(consumed) = &code.consumption else {
                continue;
            };
            let operation = &consumed.operation;
            check::operation(operation, bounds)?;
            check::source(operation, factor)?;
            if operation.kind != Kind::Replace
                || operation.generation != code.generation
                || operation.slot != Some(code.slot)
                || operation.verified_at < current.activated_at
            {
                return Err(SessionError::CorruptStore);
            }
            covered(operation, control)?;
            check::receipts(
                operation,
                consumed.intent_receipt.as_deref(),
                consumed.outcome_receipt.as_deref(),
                bounds,
            )?;
            if let Some(pending) = pending.filter(|p| p.operation.sequence == operation.sequence) {
                if pending != consumed {
                    return Err(SessionError::CorruptStore);
                }
                matched_pending = true;
            } else if consumed.intent_receipt.is_none() || consumed.outcome_receipt.is_none() {
                return Err(SessionError::CorruptStore);
            }
            used_mask |= 1_u16 << code.slot;
            registry.pending(consumed)?;
        } else if let Some(staged) = staged.filter(|p| p.operation.generation == code.generation) {
            staged_count += 1;
            if staged.operation.hashes.get(usize::from(code.slot)) != Some(&code.digest)
                || code.consumption.is_some()
            {
                return Err(SessionError::CorruptStore);
            }
        } else {
            return Err(SessionError::CorruptStore);
        }
    }
    if current_count != if current.is_some() { 10 } else { 0 }
        || staged_count != if staged.is_some() { 10 } else { 0 }
        || used_mask != control.used_mask
        || (pending.is_some_and(|p| p.operation.kind == Kind::Replace) && !matched_pending)
    {
        return Err(SessionError::CorruptStore);
    }
    Ok(())
}
