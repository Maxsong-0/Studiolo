//! 消费仅收紧权限；发布仅由原调用在最终重验后执行。

use super::super::{SessionError, database::unavailable};
use super::{
    model::{Kind, Operation, Pending},
    store,
};
use rusqlite::{Connection, params};

pub(super) fn stage(db: &Connection, operation: &Operation) -> Result<(), SessionError> {
    match operation.kind {
        Kind::Rotate => {
            for (slot, hash) in operation.hashes.iter().enumerate() {
                db.execute(
                    "INSERT INTO recovery_codes(generation,slot,digest) VALUES(?1,?2,?3)",
                    params![operation.generation, slot as i64, hash],
                )
                .map_err(unavailable)?;
            }
        }
        Kind::Replace => {
            let slot = operation.slot.ok_or(SessionError::CorruptStore)?;
            let target = operation
                .target
                .as_ref()
                .ok_or(SessionError::CorruptStore)?;
            if db.execute("UPDATE recovery_codes SET consumption_json=?1 WHERE generation=?2 AND slot=?3 AND consumption_json IS NULL",
                params![operation.encoded()?,operation.generation,slot]).map_err(unavailable)?!=1 {
                return Err(SessionError::CorruptStore);
            }
            db.execute("UPDATE recovery_control SET used_mask=used_mask|?1,high_step=?2,consumption_json=?3 WHERE id=1",
                params![1_i64<<slot,target.step,operation.encoded()?]).map_err(unavailable)?;
        }
    }
    store::insert_pending(db, operation)
}

pub(super) fn activate(db: &Connection, pending: &Pending, now: i64) -> Result<(), SessionError> {
    let proof = store::proof(pending, now)?;
    match proof.operation.kind {
        Kind::Rotate => {
            db.execute(
                "DELETE FROM recovery_codes WHERE generation!=?1",
                [proof.operation.generation],
            )
            .map_err(unavailable)?;
            db.execute(
                "UPDATE recovery_control SET current_json=?1,used_mask=0 WHERE id=1",
                [proof.encoded()?],
            )
            .map_err(unavailable)?;
        }
        Kind::Replace => {
            db.execute("DELETE FROM recovery_codes", [])
                .map_err(unavailable)?;
            db.execute("UPDATE recovery_control SET current_json=NULL,used_mask=0,change_json=?1 WHERE id=1",[proof.encoded()?]).map_err(unavailable)?;
        }
    }
    db.execute("DELETE FROM recovery_pending WHERE id=1", [])
        .map_err(unavailable)?;
    Ok(())
}

pub(super) fn discard(db: &Connection, pending: &Pending) -> Result<(), SessionError> {
    if pending.operation.kind == Kind::Rotate {
        db.execute(
            "DELETE FROM recovery_codes WHERE generation=?1",
            [pending.operation.generation],
        )
        .map_err(unavailable)?;
    }
    // Replace 的已消费码及高水位仍保留；补记绝不让失败操作重新获得权限。
    db.execute("DELETE FROM recovery_pending WHERE id=1", [])
        .map_err(unavailable)?;
    Ok(())
}
