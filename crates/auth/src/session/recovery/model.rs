//! 固定决定与有界证明；审计白名单不携带码摘要或任何种子信息。

use super::super::{SessionError, events, totp::model::Operation as FactorOperation};
use serde::{Deserialize, Serialize};
use serde_json::json;
use studiolo_audit_ledger::{AuditEvent, AuditPhase};

pub(super) const MAX_OPERATION: usize = 4096;
pub(super) const MAX_PROOF: usize = 65_536;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Kind {
    Rotate,
    Replace,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Operation {
    pub sequence: i64,
    pub kind: Kind,
    pub session_id: String,
    pub epoch: i64,
    pub generation: i64,
    pub source_enrollment: i64,
    pub source_digest: String,
    pub verified_at: i64,
    pub deadline: i64,
    pub hashes: Vec<String>,
    pub slot: Option<u8>,
    pub target: Option<FactorOperation>,
}

impl Operation {
    pub fn event(&self, store: &str, phase: AuditPhase) -> Result<AuditEvent, SessionError> {
        match self.kind {
            Kind::Replace => self
                .target
                .as_ref()
                .ok_or(SessionError::CorruptStore)?
                .event(store, phase),
            Kind::Rotate => events::event(
                &format!("{store}:recovery:{}", self.sequence),
                phase,
                "recovery.codes_rotated",
                json!({"operation":self.sequence,"generation":self.generation,
                    "session_id":self.session_id,"identity_version":self.epoch,"count":10,
                    "verified_at_ms":self.verified_at,"expires_at_ms":self.deadline,
                    "decision":if phase==AuditPhase::Intent {"requested"} else {"allow"},
                    "publication":"not_released"}),
            ),
        }
    }

    pub fn encoded(&self) -> Result<String, SessionError> {
        encode(self, MAX_OPERATION)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Proof {
    pub operation: Operation,
    pub intent_receipt: String,
    pub outcome_receipt: String,
    pub activated_at: i64,
}

impl Proof {
    pub fn encoded(&self) -> Result<String, SessionError> {
        encode(self, MAX_PROOF)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Pending {
    pub operation: Operation,
    pub intent_receipt: Option<String>,
    pub outcome_receipt: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Code {
    pub generation: i64,
    pub slot: u8,
    pub digest: String,
    pub consumption: Option<Pending>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Control {
    pub next: i64,
    pub current: Option<Proof>,
    pub changed: Option<Proof>,
    pub high_step: i64,
    pub consumption: Option<Operation>,
    pub used_mask: u16,
}

pub(super) fn decode<T>(value: String, maximum: usize) -> Result<T, SessionError>
where
    T: serde::de::DeserializeOwned + Serialize,
{
    if value.len() > maximum {
        return Err(SessionError::CorruptStore);
    }
    let parsed = serde_json::from_str(&value).map_err(|_| SessionError::CorruptStore)?;
    if encode(&parsed, maximum)? != value {
        return Err(SessionError::CorruptStore);
    }
    Ok(parsed)
}

fn encode<T: Serialize>(value: &T, maximum: usize) -> Result<String, SessionError> {
    let encoded = serde_json::to_string(value).map_err(|_| SessionError::CorruptStore)?;
    if encoded.len() > maximum {
        return Err(SessionError::CorruptStore);
    }
    Ok(encoded)
}
