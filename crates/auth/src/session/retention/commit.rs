//! 最终 CAS 同事务删除原候选、替换锚点并清除计划；不接触冻结的 legacy 摘要集。

use super::{
    RetentionReport, SessionError, checkpoint,
    plan::{self, Plan},
    selection,
};
use crate::session::{database::unavailable, schema, validation};
use rusqlite::{Connection, params};
use std::collections::BTreeSet;

pub(super) fn apply(
    db: &Connection,
    expected: &Plan,
    receipt: &str,
) -> Result<RetentionReport, SessionError> {
    if plan::completed(db, expected)? {
        return Ok(RetentionReport {
            reclaimed_sessions: 0,
            remaining: selection::remaining(db)?,
        });
    }
    if plan::read(db)?.as_ref() != Some(expected) {
        return Err(SessionError::CorruptStore);
    }
    plan::check_current(db, expected)?;
    let checkpoint = expected.checkpoint()?;
    validation::check_receipt(receipt, &checkpoint.event(&schema::store_id(db)?)?)?;
    let mut removed_events = BTreeSet::new();
    for candidate in &expected.candidates {
        for proof in &candidate.proofs {
            let (kind, event_id): (String, String) = db
                .query_row(
                    "SELECT kind,event_id FROM audit_outbox WHERE sequence=?1",
                    [proof.sequence],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .map_err(unavailable)?;
            // all-revoke 同时是其他行和连续 epoch 后缀的证据，仅允许按冻结前缀删除。
            if kind != "revoke" || !event_id.contains(":revoke-all:") {
                removed_events.insert(proof.sequence);
            }
        }
        db.execute("DELETE FROM session_seeds WHERE id=?1", [&candidate.id])
            .map_err(unavailable)?;
        if db
            .execute(
                "DELETE FROM sessions WHERE id=?1 AND state IN ('revoked','expired','abandoned')",
                [&candidate.id],
            )
            .map_err(unavailable)?
            != 1
        {
            return Err(SessionError::CorruptStore);
        }
    }
    removed_events.extend(expected.prefix.iter().map(|proof| proof.sequence));
    for sequence in removed_events {
        if db.execute("DELETE FROM audit_outbox WHERE sequence=?1 AND receipt_json IS NOT NULL AND bucket IS NULL",[sequence]).map_err(unavailable)?!=1 {
            return Err(SessionError::CorruptStore);
        }
    }
    checkpoint::replace(db, &checkpoint, receipt)?;
    db.execute(
        "DELETE FROM auth_retention_plan WHERE id=1 AND manifest_hash=?1",
        params![checkpoint.manifest_hash],
    )
    .map_err(unavailable)?;
    // 删除后的全部新格式不变量必须在同一事务内成立，失败则回滚整个批次。
    schema::validate(
        db,
        &db.query_row("SELECT config FROM auth_meta WHERE id=1", [], |row| {
            row.get::<_, String>(0)
        })
        .map_err(unavailable)?,
    )?;
    Ok(RetentionReport {
        reclaimed_sessions: expected.candidates.len(),
        remaining: selection::remaining(db)?,
    })
}
