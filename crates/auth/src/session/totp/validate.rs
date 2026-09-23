//! 单消费锚点与登记、待确认决定和会话证明之间的完整关联；不据锚点授予权限。

#[path = "validate_operation.rs"]
mod check;

use crate::session::{
    SessionError,
    database::required_row,
    schema,
    totp::{
        model::{Factor, Kind, Operation},
        store,
    },
};
use rusqlite::Connection;
use std::collections::BTreeSet;

pub(super) fn all(db: &Connection) -> Result<(), SessionError> {
    let (floor, epoch): (i64, i64) = db
        .query_row(
            "SELECT clock_floor,epoch FROM auth_meta WHERE id=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(required_row)?;
    let bounds = check::Bounds {
        version: schema::version(db)?,
        floor,
        epoch,
        next: store::next(db)?,
        store_id: schema::store_id(db)?,
    };
    let factor = store::factor(db)?;
    let pending = store::pending(db)?;
    let grants = store::grants(db)?;
    let Some(factor) = factor else {
        return if pending.is_none() && grants.is_empty() {
            Ok(())
        } else {
            Err(SessionError::CorruptStore)
        };
    };
    check::operation(&factor.consumption, &factor, &bounds)?;
    if factor.last_step != factor.consumption.step {
        return Err(SessionError::CorruptStore);
    }
    let mut sequences = BTreeSet::new();
    if let Some(enrollment) = &factor.enrollment {
        if !matches!(
            enrollment.operation.kind,
            Kind::Enroll | Kind::Recovered { .. }
        ) {
            return Err(SessionError::CorruptStore);
        }
        check::proof(enrollment, &factor, &bounds)?;
        covered(&enrollment.operation, &factor)?;
        sequences.insert(enrollment.operation.sequence);
        if factor.consumption.kind != Kind::StepUp && factor.consumption != enrollment.operation {
            return Err(SessionError::CorruptStore);
        }
    } else if !grants.is_empty() || factor.consumption.kind != Kind::Enroll {
        return Err(SessionError::CorruptStore);
    }
    if factor.consumption.kind == Kind::StepUp {
        check::after_enrollment(&factor.consumption, &factor)?;
    }
    if let Some(pending) = pending {
        let operation = &pending.operation;
        if operation != &factor.consumption || !sequences.insert(operation.sequence) {
            return Err(SessionError::CorruptStore);
        }
        check::operation(operation, &factor, &bounds)?;
        match operation.kind {
            Kind::Enroll if factor.enrollment.is_none() => {}
            Kind::StepUp => {
                check::after_enrollment(operation, &factor)?;
                check::session(db, operation)?;
            }
            Kind::Enroll | Kind::Recovered { .. } => return Err(SessionError::CorruptStore),
        }
        check::receipts(
            operation,
            pending.intent_receipt.as_deref(),
            pending.outcome_receipt.as_deref(),
            &bounds,
        )?;
    }
    for grant in grants {
        let operation = &grant.operation;
        if operation.kind != Kind::StepUp || !sequences.insert(operation.sequence) {
            return Err(SessionError::CorruptStore);
        }
        check::proof(&grant, &factor, &bounds)?;
        check::after_enrollment(operation, &factor)?;
        check::session(db, operation)?;
        covered(operation, &factor)?;
        // store 的有界 DTO 不公开主键；反查同一 session 的规范 JSON，防止主键/内容错配。
        let stored: String = db
            .query_row(
                "SELECT operation_json FROM totp_grants WHERE session_id=?1",
                [operation
                    .session_id
                    .as_ref()
                    .ok_or(SessionError::CorruptStore)?],
                |r| r.get(0),
            )
            .map_err(required_row)?;
        if stored != operation.encoded()? {
            return Err(SessionError::CorruptStore);
        }
    }
    Ok(())
}

fn covered(operation: &Operation, factor: &Factor) -> Result<(), SessionError> {
    if operation.sequence > factor.consumption.sequence
        || operation.step > factor.consumption.step
        || (operation.sequence == factor.consumption.sequence && operation != &factor.consumption)
    {
        return Err(SessionError::CorruptStore);
    }
    Ok(())
}
