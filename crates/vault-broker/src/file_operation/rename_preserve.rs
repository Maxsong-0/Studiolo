//! 已占槽的源L0、目标stage与目标L0保全；每次发布前保持可重开引用。
use super::{record::Active, *};
use crate::{
    BrokerError, FsVaultBroker,
    blob::{file, store as blobs},
    mutation,
    writer::directory,
};
pub(crate) fn preserve(broker: &FsVaultBroker, active: &mut Active) -> Result<(), BrokerError> {
    let (to, _) = rename_model::get(active)?;
    let to = to.clone();
    let old = active
        .before
        .proof()
        .ok_or(BrokerError::CorruptWriterState)?;
    let revision = old.revision.ok_or(BrokerError::CorruptWriterState)?;
    let history = directory(&broker.data, "history")?;
    let preserved = blobs::copy(
        broker,
        &broker.root,
        &active.frozen.path,
        &history,
        &revision.to_string(),
        (&active.frozen.path, revision),
        &old.identity,
    )?;
    active.history = Some(preserved.clone());
    let request = crate::StreamMutation {
        path: to.clone(),
        precondition: crate::Precondition::MustNotExist,
        content: preserved.identity.clone(),
        idempotency_key: active.frozen.key_hash.clone(),
    };
    let (stage, target) = crate::stream::stage::create(
        broker,
        &request,
        crate::stream::stage::Input::Projection {
            root: history,
            name: blobs::logical(&revision.to_string())?,
            source_path: active.frozen.path.clone(),
            source_revision: revision,
        },
        crate::StreamLimits::new(active.frozen.limits.effective_object_limit)?,
        active.sequence,
    )?;
    if target == revision {
        return Err(BrokerError::CorruptWriterState);
    }
    let stages = directory(&broker.root, ".workbench/staging")?;
    let sealed = blobs::verify(broker, &stages, &stage.name, &to, target)?;
    active.stage = Some(stage.clone());
    active.sealed_stage = Some(sealed.clone());
    active
        .rename
        .as_mut()
        .ok_or(BrokerError::CorruptWriterState)?
        .target_revision = Some(target);
    // 先固定目标 revision/stage 引用，崩溃后不会把已发布目标 L0 变成无主对象。
    let mut state = mutation::load(broker)?;
    if state
        .file_operation
        .as_ref()
        .is_none_or(|a| a.sequence != active.sequence || a.frozen != active.frozen)
    {
        return Err(BrokerError::CaptureChanged);
    }
    state.file_operation = Some(active.clone());
    mutation::save(broker, &state)?;
    mutation::fault(broker, 83)?;
    let history = directory(&broker.data, "history")?;
    let projected = blobs::copy(
        broker,
        &stages,
        &blobs::logical(&stage.name)?,
        &history,
        &target.to_string(),
        (&to, target),
        &preserved.identity,
    )?;
    active
        .rename
        .as_mut()
        .ok_or(BrokerError::CorruptWriterState)?
        .history = Some(projected);
    active.stage = Some(stage);
    active.sealed_stage = Some(sealed);
    check(broker, active)
}
pub(crate) fn check(broker: &FsVaultBroker, active: &Active) -> Result<(), BrokerError> {
    super::store::required(broker)?;
    let (to, r) = rename_model::get(active)?;
    active.before.check(broker, &active.frozen.path)?;
    r.before.check(broker, to)?;
    let history = directory(&broker.data, "history")?;
    for proof in [active.history.as_ref(), r.history.as_ref()] {
        let proof = proof.ok_or(BrokerError::CorruptWriterState)?;
        let revision = proof.revision.ok_or(BrokerError::CorruptWriterState)?;
        file::check_file(
            &history,
            &blobs::logical(&revision.to_string())?,
            &proof.node,
        )?;
    }
    let stage = active
        .stage
        .as_ref()
        .ok_or(BrokerError::CorruptWriterState)?;
    let proof = active
        .sealed_stage
        .as_ref()
        .ok_or(BrokerError::CorruptWriterState)?;
    file::check_file(
        &directory(&broker.root, ".workbench/staging")?,
        &blobs::logical(&stage.name)?,
        &proof.node,
    )?;
    super::store::required(broker)?;
    Ok(())
}
