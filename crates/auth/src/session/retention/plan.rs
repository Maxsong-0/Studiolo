//! 单一持久计划：对象集合与前缀上界冻结后不扩张，重试沿用原许可身份。

use super::{
    BATCH, SessionError,
    candidate::{self, Candidate},
    checkpoint::{self, Checkpoint},
    current_epoch, decode, digest, encode,
    proof::{self, Proof},
    selection,
};
use crate::session::database::unavailable;
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use studiolo_audit_ledger::AuditEvent;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Plan {
    pub sequence: i64,
    pub previous_sequence: i64,
    pub previous_manifest: String,
    pub epoch_base_before: i64,
    pub epoch_base_target: i64,
    pub issuance_floor: i64,
    pub candidates: Vec<Candidate>,
    pub prefix: Vec<Proof>,
}

impl Plan {
    pub fn proofs(&self) -> Vec<Proof> {
        let mut proofs = BTreeMap::new();
        for proof in self
            .candidates
            .iter()
            .flat_map(|candidate| &candidate.proofs)
            .chain(&self.prefix)
        {
            proofs.insert(proof.sequence, proof.clone());
        }
        proofs.into_values().collect()
    }

    pub fn checkpoint(&self) -> Result<Checkpoint, SessionError> {
        Ok(Checkpoint {
            sequence: self.sequence,
            previous_sequence: self.previous_sequence,
            previous_manifest: self.previous_manifest.clone(),
            epoch_base: self.epoch_base_target,
            issuance_floor: self.issuance_floor,
            event_sequence_barrier: self.sequence - 1,
            manifest_hash: digest(encode(self)?.as_bytes()),
            reclaimed_sessions: self.candidates.len(),
        })
    }

    pub fn event(&self, store: &str) -> Result<AuditEvent, SessionError> {
        self.checkpoint()?.event(store)
    }
}

pub(super) fn read(db: &Connection) -> Result<Option<Plan>, SessionError> {
    let raw:Option<(Option<String>,Option<String>)>=db.query_row(
        "SELECT CASE WHEN octet_length(manifest_hash)=64 THEN manifest_hash END,CASE WHEN octet_length(plan_json) BETWEEN 1 AND 32768 THEN plan_json END FROM auth_retention_plan WHERE id=1",[],
        |row|Ok((row.get(0)?,row.get(1)?))).optional().map_err(unavailable)?;
    let Some((manifest, encoded)) = raw else {
        return Ok(None);
    };
    let encoded = encoded.ok_or(SessionError::CorruptStore)?;
    if manifest.as_deref() != Some(digest(encoded.as_bytes()).as_str()) {
        return Err(SessionError::CorruptStore);
    }
    let plan: Plan = decode(&encoded)?;
    if plan.sequence <= 0
        || plan.previous_sequence < 0
        || plan.candidates.len() > BATCH
        || plan.prefix.len() > 2 * BATCH
        || plan.epoch_base_before < 1
        || plan.epoch_base_target < plan.epoch_base_before
        || plan.epoch_base_target - plan.epoch_base_before > BATCH as i64
        || plan.issuance_floor < 1
        || (plan.candidates.is_empty() && plan.epoch_base_target == plan.epoch_base_before)
    {
        return Err(SessionError::CorruptStore);
    }
    Ok(Some(plan))
}

pub(super) fn completed(db: &Connection, plan: &Plan) -> Result<bool, SessionError> {
    Ok(checkpoint::read(db)?.sequence >= plan.sequence)
}

pub(super) fn check_current(db: &Connection, plan: &Plan) -> Result<(), SessionError> {
    let checkpoint = checkpoint::read(db)?;
    let (next,issuance):(i64,i64)=db.query_row("SELECT next_sequence,(SELECT next_issuance FROM credential_state WHERE id=1) FROM auth_meta WHERE id=1",[],|row|Ok((row.get(0)?,row.get(1)?))).map_err(unavailable)?;
    if plan.previous_sequence != checkpoint.sequence
        || plan.previous_manifest != checkpoint.manifest_hash
        || plan.sequence <= checkpoint.sequence
        || plan.sequence >= next
        || plan.epoch_base_before != checkpoint.epoch_base
        || plan.epoch_base_target > current_epoch(db)?
        || plan.issuance_floor < checkpoint.issuance_floor
        || plan.issuance_floor > issuance
        || plan.epoch_base_target > candidate::minimum_remaining(db, &plan.candidates)?
    {
        return Err(SessionError::CorruptStore);
    }
    let mut ids = BTreeSet::new();
    for candidate in &plan.candidates {
        if !ids.insert(&candidate.id)
            || candidate::capture(db, &candidate.id)?.as_ref() != Some(candidate)
        {
            return Err(SessionError::CorruptStore);
        }
    }
    let (target, prefix) = selection::prefix(db, plan.epoch_base_before, plan.epoch_base_target)?;
    if target != plan.epoch_base_target || prefix != plan.prefix {
        return Err(SessionError::CorruptStore);
    }
    for proof in plan.proofs() {
        if proof.sequence >= plan.sequence {
            return Err(SessionError::CorruptStore);
        }
        proof::load(db, &proof)?;
    }
    Ok(())
}
