//! 分表写入：单动作更新一行，完整重写只用于 reserve/settle。
use super::{
    BridgeError, INPUT_MAX, RECEIPT_MAX, STATE_MAX, database::error, input_codec, model::*,
    state_codec,
};
use rusqlite::{Connection, params};
pub(super) fn persist(db: &Connection, s: &State, edit: Edit, previous: u64) -> Result<()> {
    match edit {
        Edit::Full => full(db, s)?,
        Edit::Action { sequence, index } => {
            let p = s
                .pending
                .iter()
                .chain(s.last.iter())
                .find(|p| p.input.plan_sequence == sequence)
                .ok_or(BridgeError::NotFound)?;
            let bytes =
                state_codec::progress_bytes(p.actions.get(index).ok_or(BridgeError::NotFound)?)?;
            let old: i64 = db
                .query_row(
                    "SELECT length(payload) FROM bridge_actions WHERE sequence=?1 AND position=?2",
                    params![sequence as i64, index as i64],
                    |r| r.get(0),
                )
                .map_err(error)?;
            adjust(db, old, bytes.len())?;
            if db
                .execute(
                    "UPDATE bridge_actions SET payload=?3 WHERE sequence=?1 AND position=?2",
                    params![sequence as i64, index as i64, bytes],
                )
                .map_err(error)?
                != 1
            {
                return Err(BridgeError::NotFound);
            }
        }
        Edit::Summary | Edit::FuseAck => {
            let (seq, ack) = match edit {
                Edit::Summary => {
                    let p = s.pending.as_ref().ok_or(BridgeError::NotFound)?;
                    (p.input.plan_sequence, &p.summary_ack)
                }
                _ => {
                    let f = s.fuse.as_ref().ok_or(BridgeError::NotFound)?;
                    (f.input.plan_sequence, &f.ack)
                }
            };
            let old:i64=db.query_row("SELECT COALESCE(length(CAST(ack AS BLOB)),0) FROM bridge_plans WHERE sequence=?1",[seq as i64],|r|r.get(0)).map_err(error)?;
            let n = ack.as_ref().map_or(0, String::len);
            if n > RECEIPT_MAX {
                return Err(BridgeError::Budget);
            }
            adjust(db, old, n)?;
            db.execute(
                "UPDATE bridge_plans SET ack=?2 WHERE sequence=?1",
                params![seq as i64, ack],
            )
            .map_err(error)?;
        }
        Edit::Retirement => replace_meta(db, s)?,
    }
    if db
        .execute(
            "UPDATE bridge_meta SET revision=?1 WHERE id=1 AND revision=?2",
            params![s.version as i64, previous as i64],
        )
        .map_err(error)?
        != 1
    {
        return Err(BridgeError::VersionConflict);
    }
    Ok(())
}
pub(super) fn full(db: &Connection, s: &State) -> Result<()> {
    // reserve/settle 才做完整有界写；不会在逐动作路径进入此分支。
    db.execute("DELETE FROM bridge_actions", [])
        .map_err(error)?;
    db.execute("DELETE FROM bridge_plans", []).map_err(error)?;
    for (role, p) in [("pending", s.pending.as_ref()), ("last", s.last.as_ref())] {
        if let Some(p) = p {
            insert_plan(db, role, &p.input, &p.summary_ack)?;
            for (i, a) in p.actions.iter().enumerate() {
                db.execute(
                    "INSERT INTO bridge_actions VALUES(?1,?2,?3)",
                    params![
                        p.input.plan_sequence as i64,
                        i as i64,
                        state_codec::progress_bytes(a)?
                    ],
                )
                .map_err(error)?;
            }
        }
    }
    if let Some(f) = &s.fuse {
        insert_plan(db, "fuse", &f.input, &f.ack)?;
    }
    db.execute(
        "UPDATE bridge_meta SET payload=?1 WHERE id=1",
        [meta_bytes(s)?],
    )
    .map_err(error)?;
    let size = total(db)?;
    if size > STATE_MAX as i64 {
        return Err(BridgeError::Budget);
    }
    db.execute("UPDATE bridge_meta SET encoded_total=?1 WHERE id=1", [size])
        .map_err(error)?;
    Ok(())
}
fn insert_plan(
    db: &Connection,
    role: &str,
    input: &crate::Input,
    ack: &Option<String>,
) -> Result<()> {
    let bytes = input_codec::encode(input)?;
    if bytes.len() > INPUT_MAX || ack.as_ref().is_some_and(|a| a.len() > RECEIPT_MAX) {
        return Err(BridgeError::Budget);
    }
    db.execute(
        "INSERT INTO bridge_plans VALUES(?1,?2,?3,?4)",
        params![input.plan_sequence as i64, role, bytes, ack],
    )
    .map_err(error)?;
    Ok(())
}
fn replace_meta(db: &Connection, s: &State) -> Result<()> {
    let bytes = meta_bytes(s)?;
    let old: i64 = db
        .query_row("SELECT length(payload) FROM bridge_meta", [], |r| r.get(0))
        .map_err(error)?;
    adjust(db, old, bytes.len())?;
    db.execute("UPDATE bridge_meta SET payload=?1 WHERE id=1", [bytes])
        .map_err(error)?;
    Ok(())
}
pub(super) fn meta_bytes(s: &State) -> Result<Vec<u8>> {
    // revision 独立列更新；meta 内固定 0，不触发整行重写。
    state_codec::encode(&State {
        version: 0,
        binding: s.binding.clone(),
        limits: s.limits,
        cursor: s.cursor.clone(),
        baseline: s.baseline.clone(),
        pending: None,
        last: None,
        floor: s.floor,
        retirement: s.retirement.clone(),
        fuse: None,
    })
}
fn adjust(db: &Connection, old: i64, new: usize) -> Result<()> {
    if db.execute("UPDATE bridge_meta SET encoded_total=encoded_total-?1+?2 WHERE id=1 AND encoded_total>=?1 AND encoded_total-?1+?2<=67108864",params![old,new as i64]).map_err(error)?!=1{return Err(BridgeError::Budget);}
    Ok(())
}
pub(super) fn total(db: &Connection) -> Result<i64> {
    db.query_row("SELECT (SELECT length(payload)+24 FROM bridge_meta)+(SELECT COALESCE(sum(length(input)+COALESCE(length(CAST(ack AS BLOB)),0)+length(role)+8),0) FROM bridge_plans)+(SELECT COALESCE(sum(length(payload)+16),0) FROM bridge_actions)",[],|r|r.get(0)).map_err(error)
}
