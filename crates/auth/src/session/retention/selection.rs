//! 冻结最老可回收行与已确认版本前缀；选择与状态报告均有固定逻辑预算。

use super::{
    BATCH, MAX_PLAN_BYTES, RetentionWork, SessionError, candidate, checkpoint, current_epoch,
    digest, encode,
    plan::{self, Plan},
    proof::{self, Proof},
};
use crate::session::{database::unavailable, schema};
use rusqlite::{Connection, params};

pub(super) fn obtain(db: &Connection) -> Result<Option<Plan>, SessionError> {
    if let Some(plan) = plan::read(db)? {
        return Ok(Some(plan));
    }
    let checkpoint = checkpoint::read(db)?;
    let candidates = candidate::select(db)?;
    let maximum = candidate::minimum_remaining(db, &candidates)?
        .min(checkpoint.epoch_base.saturating_add(BATCH as i64));
    let (target, prefix) = prefix(db, checkpoint.epoch_base, maximum)?;
    if candidates.is_empty() && target == checkpoint.epoch_base {
        return Ok(None);
    }
    let sequence = schema::next_sequence(db)?;
    let issuance_floor = db
        .query_row(
            "SELECT next_issuance FROM credential_state WHERE id=1",
            [],
            |row| row.get(0),
        )
        .map_err(unavailable)?;
    let plan = Plan {
        sequence,
        previous_sequence: checkpoint.sequence,
        previous_manifest: checkpoint.manifest_hash,
        epoch_base_before: checkpoint.epoch_base,
        epoch_base_target: target,
        issuance_floor,
        candidates,
        prefix,
    };
    let encoded = encode(&plan)?;
    if encoded.len() > MAX_PLAN_BYTES {
        return Err(SessionError::Capacity);
    }
    db.execute(
        "INSERT INTO auth_retention_plan VALUES(1,?1,?2)",
        params![digest(encoded.as_bytes()), encoded],
    )
    .map_err(unavailable)?;
    Ok(Some(plan))
}

pub(super) fn prefix(
    db: &Connection,
    base: i64,
    maximum: i64,
) -> Result<(i64, Vec<Proof>), SessionError> {
    // base 可能已达 i64 上界；空前缀不能经饱和加法变成一次错误读取。
    if maximum <= base {
        return Ok((base, Vec::new()));
    }
    if maximum - base > BATCH as i64 {
        return Err(SessionError::CorruptStore);
    }
    let mut target = base;
    let mut proofs = Vec::new();
    for epoch in base + 1..=maximum {
        let id = crate::session::revocation::epoch_event(db, epoch)?;
        let Some(intent) = proof::for_event(db, &id, "intent")? else {
            break;
        };
        let Some(outcome) = proof::for_event(db, &id, "outcome")? else {
            break;
        };
        proofs.extend([intent, outcome]);
        target = epoch;
    }
    Ok((target, proofs))
}

pub(super) fn remaining(db: &Connection) -> Result<RetentionWork, SessionError> {
    if plan::read(db)?.is_some() || !candidate::select(db)?.is_empty() {
        return Ok(RetentionWork::More);
    }
    let base = checkpoint::read(db)?.epoch_base;
    let minimum = candidate::minimum_remaining(db, &[])?;
    if prefix(db, base, minimum.min(base.saturating_add(BATCH as i64)))?.0 > base {
        return Ok(RetentionWork::More);
    }
    let terminal: i64 = db
        .query_row(
            "SELECT count(*) FROM sessions WHERE state IN ('revoked','expired','abandoned')",
            [],
            |row| row.get(0),
        )
        .map_err(unavailable)?;
    Ok(if terminal > 0 || current_epoch(db)? > base {
        RetentionWork::AwaitingDelivery
    } else {
        RetentionWork::None
    })
}
