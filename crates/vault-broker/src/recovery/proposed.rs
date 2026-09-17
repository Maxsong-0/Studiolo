//! proposed 只有旧有界原请求或固定 stage/L0 两种可信来源，不公开任意写入回调。
use crate::blob::{BlobIdentity, file::ContentProof, store};
use crate::{BrokerError, FsVaultBroker, Revision, VaultPath};
use cap_std::fs::Dir;
use std::path::Path;

pub(crate) enum Proposed {
    Buffered {
        bytes: Vec<u8>,
        identity: BlobIdentity,
        revision: Revision,
    },
    Stored {
        root: Dir,
        path: VaultPath,
        proof: ContentProof,
    },
}
impl Proposed {
    pub(super) fn buffered(
        broker: &FsVaultBroker,
        op: &crate::mutation::record::Operation,
        bytes: Vec<u8>,
    ) -> Result<Self, BrokerError> {
        if bytes.len() > crate::writer::disk::CONTENT_LIMIT {
            return Err(BrokerError::WriterLimitExceeded);
        }
        let identity = BlobIdentity {
            bytes: bytes.len() as u64,
            sha256: crate::sha256(&bytes),
        };
        let revision = Revision::compute(broker.vault_uuid, &op.path, &bytes);
        if identity.bytes != op.stage.size
            || identity.sha256 != op.stage.digest
            || revision != op.target_revision
        {
            return Err(BrokerError::IdempotencyConflict);
        }
        Ok(Self::Buffered {
            bytes,
            identity,
            revision,
        })
    }
    pub(super) fn stored(
        broker: &FsVaultBroker,
        root: Dir,
        path: VaultPath,
        target: &VaultPath,
        revision: Revision,
    ) -> Result<Self, BrokerError> {
        let proof = store::verify(broker, &root, path.as_str(), target, revision)?;
        Ok(Self::Stored { root, path, proof })
    }
    pub(super) fn identity(&self) -> &BlobIdentity {
        match self {
            Self::Buffered { identity, .. } => identity,
            Self::Stored { proof, .. } => &proof.identity,
        }
    }
    pub(super) fn revision(&self) -> Result<Revision, BrokerError> {
        match self {
            Self::Buffered { revision, .. } => Ok(*revision),
            Self::Stored { proof, .. } => proof.revision.ok_or(BrokerError::CorruptWriterState),
        }
    }
    pub(super) fn preserve(
        &self,
        broker: &FsVaultBroker,
        output: &Dir,
        name: &str,
        target: &VaultPath,
    ) -> Result<(), BrokerError> {
        let revision = self.revision()?;
        match self {
            Self::Stored { root, path, proof } => {
                crate::blob::file::check(root, path, &proof.node)?;
                store::copy(
                    broker,
                    root,
                    path,
                    output,
                    name,
                    (target, revision),
                    &proof.identity,
                )?;
            }
            Self::Buffered { bytes, .. } => {
                match store::verify(broker, output, name, target, revision) {
                    Ok(proof) if proof.identity == *self.identity() => return Ok(()),
                    Ok(_) => return Err(BrokerError::CorruptWriterState),
                    Err(BrokerError::NotFound(_)) => {}
                    Err(error) => return Err(error),
                }
                FsVaultBroker::atomic_write_dir_checked(
                    output,
                    Path::new(name),
                    bytes,
                    true,
                    || broker.writer.check(&broker.root, &broker.data),
                )?;
                store::verify(broker, output, name, target, revision)?;
            }
        }
        Ok(())
    }
}
