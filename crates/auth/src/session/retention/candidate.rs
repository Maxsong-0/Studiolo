//! 仅终态且原始证据完整交付的会话可以成为冻结候选。

use super::{
    SessionError, digest, encode,
    proof::{self, Proof},
};
use crate::session::{database::unavailable, schema};
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Candidate {
    pub id: String,
    pub seal: String,
    pub proofs: Vec<Proof>,
}

pub(super) fn capture(db: &Connection, id: &str) -> Result<Option<Candidate>, SessionError> {
    if crate::session::totp::blocks_retention(db, id)?
        || crate::session::recovery::blocks_retention(db, id)?
    {
        return Ok(None);
    }
    let raw: Option<(String,i64,String,String)> = db.query_row(
        "SELECT id,token_digest,csrf_digest,epoch,state,created,last_seen,absolute_expiry,access_expiry,grant_id,grant_receipt,activated_at FROM sessions WHERE id=?1",
        [id], |row| Ok((row.get(4)?,row.get(3)?,row.get(9)?,proof::seal_row(row,12,b"auth-retention-session-v1\0")?)))
        .optional().map_err(unavailable)?;
    let Some((state, epoch, grant, row_seal)) = raw else {
        return Ok(None);
    };
    if !matches!(state.as_str(), "revoked" | "expired" | "abandoned") {
        return Ok(None);
    }
    let Some(grant) = proof::for_event(db, &grant, "outcome")? else {
        return Ok(None);
    };
    let store = schema::store_id(db)?;
    let mut proofs = vec![grant];
    if state == "revoked" {
        let one = format!("{store}:revoke:{id}");
        let exists: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM audit_outbox WHERE event_id=?1)",
                [&one],
                |row| row.get(0),
            )
            .map_err(unavailable)?;
        let terminal = if exists {
            one
        } else {
            crate::session::revocation::epoch_event(
                db,
                epoch.checked_add(1).ok_or(SessionError::CorruptStore)?,
            )?
        };
        for phase in ["intent", "outcome"] {
            let Some(proof) = proof::for_event(db, &terminal, phase)? else {
                return Ok(None);
            };
            proofs.push(proof);
        }
    } else {
        let Some(proof) = proof::for_event(db, &format!("{store}:expire:{id}"), "outcome")? else {
            return Ok(None);
        };
        proofs.push(proof);
    }
    proofs.sort_by_key(|proof| proof.sequence);
    let seed: Option<String> = db
        .query_row(
            "SELECT id,issuance,token_seed,csrf_seed FROM session_seeds WHERE id=?1",
            [id],
            |row| proof::seal_row(row, 4, b"auth-retention-seed-v1\0"),
        )
        .optional()
        .map_err(unavailable)?;
    let seal = digest(encode(&(row_seal, seed, &proofs))?.as_bytes());
    Ok(Some(Candidate {
        id: id.to_owned(),
        seal,
        proofs,
    }))
}

pub(super) fn select(db: &Connection) -> Result<Vec<Candidate>, SessionError> {
    let mut statement=db.prepare("SELECT id FROM sessions WHERE state IN ('revoked','expired','abandoned') ORDER BY epoch,created,id").map_err(unavailable)?;
    let ids = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(unavailable)?;
    let mut candidates = Vec::new();
    for id in ids {
        if let Some(candidate) = capture(db, &id.map_err(unavailable)?)? {
            candidates.push(candidate);
            if candidates.len() == super::BATCH {
                break;
            }
        }
    }
    Ok(candidates)
}

pub(super) fn minimum_remaining(
    db: &Connection,
    candidates: &[Candidate],
) -> Result<i64, SessionError> {
    let mut minimum = super::current_epoch(db)?;
    let mut statement = db
        .prepare("SELECT id,epoch FROM sessions")
        .map_err(unavailable)?;
    for row in statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .map_err(unavailable)?
    {
        let (id, epoch) = row.map_err(unavailable)?;
        if !candidates.iter().any(|candidate| candidate.id == id) {
            minimum = minimum.min(epoch);
        }
    }
    Ok(minimum)
}
