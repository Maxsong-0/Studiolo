//! 严格区分缺失、普通文件和不可观测；持久证据只保存摘要与节点身份，不存正文。
use super::model::FilePresence;
use crate::{
    BrokerError, FsVaultBroker, VaultPath,
    blob::{current, file},
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Observation {
    Missing { parents: Vec<(String, file::Stamp)> },
    Present { proof: file::ContentProof },
}
impl Observation {
    pub(crate) fn observe(broker: &FsVaultBroker, path: &VaultPath) -> Result<Self, BrokerError> {
        Ok(match current::observe(broker, path)? {
            current::Current::Missing(parents) => Self::Missing { parents },
            current::Current::Present(proof) => Self::Present { proof },
        })
    }
    pub(crate) fn check(
        &self,
        broker: &FsVaultBroker,
        path: &VaultPath,
    ) -> Result<(), BrokerError> {
        let expected = match self {
            Self::Missing { parents } => current::Current::Missing(parents.clone()),
            Self::Present { proof } => current::Current::Present(proof.clone()),
        };
        current::check(broker, path, &expected)
    }
    pub(crate) fn presence(&self) -> Result<FilePresence, BrokerError> {
        Ok(match self {
            Self::Missing { .. } => FilePresence::Missing,
            Self::Present { proof } => FilePresence::Present {
                revision: proof.revision.ok_or(BrokerError::CorruptWriterState)?,
                sha256: proof.identity.sha256.clone(),
                bytes: proof.identity.bytes,
            },
        })
    }
    pub(crate) fn proof(&self) -> Option<&file::ContentProof> {
        match self {
            Self::Missing { .. } => None,
            Self::Present { proof } => Some(proof),
        }
    }
    pub(crate) fn validate(&self) -> Result<(), BrokerError> {
        let parents = match self {
            Self::Missing { parents } => parents,
            Self::Present { proof } => {
                proof.identity.validate()?;
                if proof.revision.is_none() || proof.node.file.bytes != proof.identity.bytes {
                    return Err(BrokerError::CorruptWriterState);
                }
                &proof.node.parents
            }
        };
        if parents.is_empty() || parents.len() > 64 || !parents[0].0.is_empty() {
            return Err(BrokerError::CorruptWriterState);
        }
        Ok(())
    }
}
