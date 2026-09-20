//! 固定阶段日志和按键终态；只有 Completed 记录能证明本协议完成过安装。

use crate::writer::{Identity, disk};
use crate::{BrokerError, Mutation, MutationReceipt, Precondition, Revision, VaultPath, VaultUuid};
use serde::{Deserialize, Serialize};

mod state;
pub(crate) use state::State;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Phase {
    Prepared,
    Installing,
    Completed,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Stage {
    pub(crate) name: String,
    pub(crate) identity: Identity,
    pub(crate) digest: String,
    pub(crate) size: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Operation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) recording: Option<crate::recorded::store::RecordingRef>,
    pub(crate) sequence: u64,
    pub(crate) key_hash: String,
    pub(crate) fingerprint: String,
    pub(crate) path: VaultPath,
    pub(crate) precondition: Precondition,
    pub(crate) target_revision: Revision,
    pub(crate) previous_revision: Option<Revision>,
    pub(crate) stage: Stage,
    pub(crate) phase: Phase,
    pub(crate) receipt: Option<MutationReceipt>,
}

impl Operation {
    pub(crate) fn validate(&self, streaming: bool) -> Result<(), BrokerError> {
        if self.sequence == 0
            || self
                .recording
                .as_ref()
                .is_some_and(|r| !disk::hex(&r.id) || !disk::hex(&r.digest))
            || !disk::hex(&self.key_hash)
            || !disk::hex(&self.fingerprint)
            || self.path.is_protected()
            || !disk::hex(&self.stage.digest)
            || self.stage.size
                > (if streaming {
                    crate::blob::OBJECT_LIMIT
                } else {
                    disk::CONTENT_LIMIT as u64
                })
            || !disk::hex(self.stage.name.strip_suffix(".stage").unwrap_or(""))
            || self.previous_revision
                != match self.precondition {
                    Precondition::MustNotExist => None,
                    Precondition::Match(revision) => Some(revision),
                }
        {
            return Err(BrokerError::CorruptWriterState);
        }
        match (&self.receipt, self.phase) {
            (Some(receipt), Phase::Completed) if receipt == &self.receipt() => Ok(()),
            (None, Phase::Prepared | Phase::Installing) => Ok(()),
            _ => Err(BrokerError::CorruptWriterState),
        }
    }
    pub(crate) fn receipt(&self) -> MutationReceipt {
        MutationReceipt {
            path: self.path.clone(),
            revision: self.target_revision,
            previous_revision: self.previous_revision,
        }
    }
    pub(crate) fn matches(&self, mutation: &Mutation, fingerprint: &str, target: Revision) -> bool {
        self.key_hash == crate::sha256(mutation.idempotency_key.as_bytes())
            && self.fingerprint == fingerprint
            && self.path == mutation.path
            && self.precondition == mutation.precondition
            && self.target_revision == target
            && self.stage.digest == crate::sha256(&mutation.content)
            && self.stage.size == mutation.content.len() as u64
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Terminal {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) recording: Option<crate::recorded::store::RecordingRef>,
    pub(crate) format_version: u32,
    pub(crate) pair_id: VaultUuid,
    pub(crate) sequence: Option<u64>,
    pub(crate) fingerprint: String,
    pub(crate) expected_revision: Revision,
    pub(crate) outcome: Outcome,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Outcome {
    Committed {
        receipt: MutationReceipt,
    },
    Rejected {
        path: VaultPath,
        precondition: Precondition,
        actual: Option<Revision>,
    },
    Cancelled {
        path: VaultPath,
        precondition: Precondition,
    },
    Indeterminate {
        path: VaultPath,
        precondition: Precondition,
        evidence_id: String,
        current_revision: Option<Revision>,
    },
}

impl Terminal {
    pub(crate) fn validate(&self, pair: VaultUuid) -> Result<(), BrokerError> {
        if !matches!(self.format_version, 2..=4)
            || (self.format_version == 4) != self.recording.is_some()
            || self
                .recording
                .as_ref()
                .is_some_and(|r| !disk::hex(&r.id) || !disk::hex(&r.digest))
            || self.pair_id != pair
            || !disk::hex(&self.fingerprint)
        {
            return Err(BrokerError::CorruptWriterState);
        }
        let valid = match &self.outcome {
            Outcome::Committed { receipt } => {
                matches!(self.format_version, 2 | 4)
                    && self.sequence.is_some_and(|s| s > 0)
                    && receipt.revision == self.expected_revision
                    && !receipt.path.is_protected()
            }
            Outcome::Rejected {
                path,
                precondition,
                actual,
            } => {
                matches!(self.format_version, 2 | 4)
                    && self.sequence.is_none()
                    && !path.is_protected()
                    && !cas_matches(*precondition, *actual)
            }
            Outcome::Cancelled { path, .. } => {
                matches!(self.format_version, 3 | 4)
                    && self.sequence.is_none()
                    && !path.is_protected()
            }
            Outcome::Indeterminate {
                path, evidence_id, ..
            } => {
                matches!(self.format_version, 3 | 4)
                    && self.sequence.is_none()
                    && !path.is_protected()
                    && disk::hex(evidence_id)
            }
        };
        if valid {
            Ok(())
        } else {
            Err(BrokerError::CorruptWriterState)
        }
    }
    pub(crate) fn result(self) -> Result<MutationReceipt, BrokerError> {
        self.outcome.result()
    }
}

impl Outcome {
    pub(crate) fn result(self) -> Result<MutationReceipt, BrokerError> {
        match self {
            Outcome::Committed { receipt } => Ok(receipt),
            Outcome::Rejected {
                path,
                precondition,
                actual,
            } => Err(conflict(path, precondition, actual)),
            Outcome::Cancelled { .. } => Err(BrokerError::MutationCancelled),
            Outcome::Indeterminate { evidence_id, .. } => {
                Err(BrokerError::MutationIndeterminate { evidence_id })
            }
        }
    }
}

pub(crate) fn cas_matches(precondition: Precondition, actual: Option<Revision>) -> bool {
    match precondition {
        Precondition::MustNotExist => actual.is_none(),
        Precondition::Match(expected) => actual == Some(expected),
    }
}

pub(crate) fn conflict(
    path: VaultPath,
    precondition: Precondition,
    actual: Option<Revision>,
) -> BrokerError {
    BrokerError::RevisionConflict {
        path,
        actual,
        expected: match precondition {
            Precondition::MustNotExist => "absence".to_owned(),
            Precondition::Match(r) => r.to_string(),
        },
    }
}
