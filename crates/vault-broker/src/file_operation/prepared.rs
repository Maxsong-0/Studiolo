//! 完整流式保全与票据核验分开：commit 前只核对固定身份与元数据，不追加大读。
use super::{record::Active, tombstone::Tombstone, *};
use crate::{
    BrokerError, FsVaultBroker,
    blob::{file, store as blobs},
    writer::directory,
};

pub(crate) fn preserve(
    broker: &FsVaultBroker,
    active: &mut Active,
    source: Option<&Tombstone>,
) -> Result<(), BrokerError> {
    if active.rename.is_some() {
        return super::rename_prepare::preserve(broker, active);
    }
    let history = directory(&broker.data, "history")?;
    let path = &active.frozen.path;
    let (revision, identity) = match &active.frozen.kind {
        FileOperationKind::Rename { .. } => return Err(BrokerError::CorruptWriterState),
        FileOperationKind::Delete { expected } => {
            let old = active
                .before
                .proof()
                .ok_or(BrokerError::CorruptWriterState)?;
            let proof = blobs::copy(
                broker,
                &broker.root,
                path,
                &history,
                &expected.to_string(),
                (path, *expected),
                &old.identity,
            )?;
            active.history = Some(proof);
            (*expected, old.identity.clone())
        }
        FileOperationKind::RestoreTombstone { .. } => {
            let source = source.ok_or(BrokerError::CorruptWriterState)?;
            let entry = source.entry()?;
            let preserved = blobs::verify(
                broker,
                &history,
                &entry.revision.to_string(),
                path,
                entry.revision,
            )?;
            if preserved.identity != source.history.identity {
                return Err(BrokerError::CorruptWriterState);
            }
            active.history = Some(preserved);
            (entry.revision, source.history.identity.clone())
        }
    };
    if matches!(
        active.frozen.kind,
        FileOperationKind::RestoreTombstone { .. }
    ) {
        let request = crate::StreamMutation {
            path: path.clone(),
            precondition: crate::Precondition::MustNotExist,
            content: identity,
            idempotency_key: active.frozen.key_hash.clone(),
        };
        let (stage, target) = crate::stream::stage::create(
            broker,
            &request,
            crate::stream::stage::Input::History {
                root: history,
                name: blobs::logical(&revision.to_string())?,
                revision,
            },
            crate::StreamLimits::new(active.frozen.limits.effective_object_limit)?,
            active.sequence,
        )?;
        if target != revision {
            return Err(BrokerError::CorruptWriterState);
        }
        let stages = directory(&broker.root, ".workbench/staging")?;
        active.sealed_stage = Some(blobs::verify(broker, &stages, &stage.name, path, revision)?);
        active.stage = Some(stage);
    }
    check(broker, active)
}
pub(crate) fn check(broker: &FsVaultBroker, active: &Active) -> Result<(), BrokerError> {
    if active.rename.is_some() {
        return super::rename_prepare::check(broker, active);
    }
    super::store::required(broker)?;
    active.before.check(broker, &active.frozen.path)?;
    let history = directory(&broker.data, "history")?;
    let preserved = active
        .history
        .as_ref()
        .ok_or(BrokerError::CorruptWriterState)?;
    let revision = preserved.revision.ok_or(BrokerError::CorruptWriterState)?;
    file::check_file(
        &history,
        &blobs::logical(&revision.to_string())?,
        &preserved.node,
    )?;
    if let Some(stage) = &active.stage {
        let stages = directory(&broker.root, ".workbench/staging")?;
        let sealed = active
            .sealed_stage
            .as_ref()
            .ok_or(BrokerError::CorruptWriterState)?;
        file::check_file(&stages, &blobs::logical(&stage.name)?, &sealed.node)?;
        if sealed.node.file.identity != stage.identity
            || sealed.identity.bytes != stage.size
            || sealed.identity.sha256 != stage.digest
        {
            return Err(BrokerError::CorruptWriterState);
        }
    }
    super::store::required(broker)?;
    Ok(())
}
