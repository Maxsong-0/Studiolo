//! Receiving 只存在于独占 &mut Broker 的调用内；失败残留计入完整 stage 配额。
use super::StreamMutation;
use crate::blob::{file, quota, store};
use crate::mutation::record::Stage;
use crate::writer::{Identity, directory};
use crate::{BlobSource, BrokerError, FsVaultBroker, StreamLimits, VaultPath};
use cap_std::fs::{Dir, OpenOptions};
use std::path::Path;

pub(crate) enum Input<'a> {
    External(&'a BlobSource),
    Projection {
        root: Dir,
        name: VaultPath,
        source_path: VaultPath,
        source_revision: crate::Revision,
    },
    History {
        root: Dir,
        name: VaultPath,
        revision: crate::Revision,
    },
}
pub(crate) fn create(
    broker: &FsVaultBroker,
    request: &StreamMutation,
    input: Input<'_>,
    limits: StreamLimits,
    sequence: u64,
) -> Result<(Stage, crate::Revision), BrokerError> {
    quota::reserve(broker, request.content.bytes)?;
    let dir = directory(&broker.root, ".workbench/staging")?;
    let name = format!(
        "{}.stage",
        crate::sha256(
            format!(
                "{}:{sequence}:{}",
                broker.writer.binding.pair_id,
                crate::VaultUuid::new()
            )
            .as_bytes()
        )
    );
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    let mut output = dir
        .open_with(&name, &options)
        .map_err(|e| BrokerError::io("create stream stage", e))?;
    let source = match &input {
        Input::External(source) => source.transfer(
            &mut output,
            limits,
            broker.vault_uuid,
            &request.path,
            &request.content,
        )?,
        Input::Projection {
            root,
            name,
            source_path,
            source_revision,
        } => {
            let old = store::verify(broker, root, name.as_str(), source_path, *source_revision)?;
            let value = file::transfer(
                root,
                name,
                Some(&mut output),
                request.content.bytes,
                Some((broker.vault_uuid, &request.path)),
            )?;
            if old.identity != request.content
                || value.identity != old.identity
                || value.node.file != old.node.file
            {
                return Err(BrokerError::CaptureChanged);
            }
            file::check_file(root, name, &old.node)?;
            value
        }
        Input::History {
            root,
            name,
            revision,
        } => {
            let value = file::transfer(
                root,
                name,
                Some(&mut output),
                request.content.bytes,
                Some((broker.vault_uuid, &request.path)),
            )?;
            if value.identity != request.content || value.revision != Some(*revision) {
                return Err(BrokerError::CorruptWriterState);
            }
            value
        }
    };
    output
        .sync_all()
        .map_err(|e| BrokerError::io("sync stream stage", e))?;
    FsVaultBroker::sync_directory(&dir, Path::new("."))?;
    let revision = source.revision.ok_or(BrokerError::CorruptWriterState)?;
    let sealed = store::verify(broker, &dir, &name, &request.path, revision)?;
    let identity = Identity::file(
        &output
            .metadata()
            .map_err(|e| BrokerError::io("stat stream stage", e))?,
    );
    if sealed.identity != request.content || sealed.node.file.identity != identity {
        return Err(BrokerError::CaptureChanged);
    }
    match &input {
        Input::External(value) => value.check(&source)?,
        Input::History { root, name, .. } | Input::Projection { root, name, .. } => {
            file::check(root, name, &source.node)?
        }
    }
    broker.writer.check(&broker.root, &broker.data)?;
    Ok((
        Stage {
            name,
            identity,
            digest: sealed.identity.sha256,
            size: sealed.identity.bytes,
        },
        revision,
    ))
}
