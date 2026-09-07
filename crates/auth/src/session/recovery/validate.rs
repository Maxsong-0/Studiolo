//! 恢复证明仅能关联真实已登记因子；永久消费锚点不要求旧会话或旧码批仍存在。

#[path = "validate_operation.rs"]
mod check;
#[path = "validate_links.rs"]
mod links;

use crate::session::{
    SessionError,
    recovery::{model::Kind, store},
    totp::{
        model::{Factor, Kind as FactorKind},
        store as factor_store,
    },
};
use rusqlite::Connection;

pub(super) fn all(db: &Connection) -> Result<(), SessionError> {
    let control = store::control(db)?;
    let pending = store::pending(db)?;
    let codes = store::codes(db)?;
    let bounds = check::Bounds::load(db, control.next)?;
    let factor = factor_store::factor(db)?;
    let Some(factor) = factor else {
        return if control.current.is_none()
            && control.changed.is_none()
            && control.consumption.is_none()
            && control.high_step == -1
            && control.used_mask == 0
            && pending.is_none()
            && codes.is_empty()
        {
            Ok(())
        } else {
            Err(SessionError::CorruptStore)
        };
    };
    let mut registry = links::Registry::default();
    if let Some(anchor) = &control.consumption {
        check::operation(anchor, &bounds)?;
        if anchor.kind != Kind::Replace
            || anchor
                .target
                .as_ref()
                .is_none_or(|target| target.step != control.high_step)
        {
            return Err(SessionError::CorruptStore);
        }
        registry.add(anchor, None)?;
    } else if control.high_step != -1 {
        return Err(SessionError::CorruptStore);
    }
    if let Some(changed) = &control.changed {
        if changed.operation.kind != Kind::Replace {
            return Err(SessionError::CorruptStore);
        }
        check::proof(changed, &bounds)?;
        links::covered(&changed.operation, &control)?;
        let enrollment = factor
            .enrollment
            .as_ref()
            .ok_or(SessionError::CorruptStore)?;
        if changed.operation.target.as_ref() != Some(&enrollment.operation)
            || changed.intent_receipt != enrollment.intent_receipt
            || changed.outcome_receipt != enrollment.outcome_receipt
            || changed.activated_at != enrollment.activated_at
        {
            return Err(SessionError::CorruptStore);
        }
        registry.add(
            &changed.operation,
            Some((
                Some(&changed.intent_receipt),
                Some(&changed.outcome_receipt),
            )),
        )?;
    } else if factor
        .enrollment
        .as_ref()
        .is_some_and(|proof| matches!(proof.operation.kind, FactorKind::Recovered { .. }))
    {
        return Err(SessionError::CorruptStore);
    }
    if let Some(current) = &control.current {
        if current.operation.kind != Kind::Rotate {
            return Err(SessionError::CorruptStore);
        }
        check::proof(current, &bounds)?;
        check::source(&current.operation, &factor)?;
        if control
            .changed
            .as_ref()
            .is_some_and(|changed| current.operation.sequence <= changed.operation.sequence)
        {
            return Err(SessionError::CorruptStore);
        }
        registry.add(
            &current.operation,
            Some((
                Some(&current.intent_receipt),
                Some(&current.outcome_receipt),
            )),
        )?;
    }
    if let Some(pending) = &pending {
        let operation = &pending.operation;
        check::operation(operation, &bounds)?;
        check::source(operation, &factor)?;
        check::session(db, operation)?;
        check::receipts(
            operation,
            pending.intent_receipt.as_deref(),
            pending.outcome_receipt.as_deref(),
            &bounds,
        )?;
        if factor_store::pending(db)?.is_some()
            || control
                .current
                .as_ref()
                .is_some_and(|p| operation.sequence <= p.operation.sequence)
            || control
                .changed
                .as_ref()
                .is_some_and(|p| operation.sequence <= p.operation.sequence)
        {
            return Err(SessionError::CorruptStore);
        }
        match operation.kind {
            Kind::Replace if control.consumption.as_ref() == Some(operation) => {}
            Kind::Rotate => {}
            Kind::Replace => return Err(SessionError::CorruptStore),
        }
        registry.pending(pending)?;
    }
    links::codes(
        &codes,
        &control,
        pending.as_ref(),
        &factor,
        &bounds,
        &mut registry,
    )?;
    registry.consumption_order()?;
    if pending
        .as_ref()
        .is_some_and(|p| p.operation.sequence != registry.maximum())
    {
        return Err(SessionError::CorruptStore);
    }
    unused_factor(&factor, &control)?;
    Ok(())
}

fn unused_factor(
    factor: &Factor,
    control: &crate::session::recovery::model::Control,
) -> Result<(), SessionError> {
    if factor.enrollment.is_none()
        && (control.current.is_some() || control.changed.is_some() || control.consumption.is_some())
    {
        return Err(SessionError::CorruptStore);
    }
    Ok(())
}
