//! 未决全批阻止所有公开读写；重开只在每项证据成立时前滚或继续整批回滚。
use super::record;
use crate::mutation::record::{Operation, Phase};
use crate::{
    BrokerError, FsVaultBroker, Revision, mutation,
    writer::{directory, disk},
};
use std::path::Path;

pub(super) fn require_installed(
    broker: &FsVaultBroker,
    item: &Operation,
) -> Result<(), BrokerError> {
    let current = crate::blob::current::observe(broker, &item.path)?;
    match current {
        crate::blob::current::Current::Present(proof)
            if proof.revision == Some(item.target_revision)
                && proof.node.file.identity == item.stage.identity
                && proof.identity.sha256 == item.stage.digest
                && proof.identity.bytes == item.stage.size =>
        {
            Ok(())
        }
        other => Err(crate::mutation::record::conflict(
            item.path.clone(),
            crate::Precondition::Match(item.target_revision),
            other.revision(),
        )),
    }
}
pub(super) fn original(
    broker: &FsVaultBroker,
    item: &Operation,
) -> Result<Option<Vec<u8>>, BrokerError> {
    item.previous_revision
        .map(|revision| {
            let history = directory(&broker.data, "history")?;
            let bytes = disk::read(&history, &revision.to_string(), disk::CONTENT_LIMIT)?
                .ok_or(BrokerError::CorruptWriterState)?;
            if Revision::compute(broker.vault_uuid, &item.path, &bytes) != revision {
                return Err(BrokerError::CorruptWriterState);
            }
            Ok(bytes)
        })
        .transpose()
}
fn restored(broker: &FsVaultBroker, item: &Operation) -> Result<bool, BrokerError> {
    Ok(broker.inspect_existing(&item.path)?.map(|v| v.revision) == item.previous_revision)
}
pub(super) fn check(broker: &FsVaultBroker, active: &record::Active) -> Result<(), BrokerError> {
    for item in &active.items {
        if active.rollback {
            original(broker, item)?;
            if !restored(broker, item)? {
                require_installed(broker, item)?;
            }
        } else if require_installed(broker, item).is_err() {
            mutation::install::check_before_install(broker, item)?;
        }
    }
    Ok(())
}
pub(crate) fn resume(
    broker: &FsVaultBroker,
    state: &mut mutation::State,
) -> Result<(), BrokerError> {
    let Some(mut active) = state.batch.clone() else {
        return Ok(());
    };
    if crate::maintenance::blocked(broker)? {
        return Ok(());
    }
    // 已观察到冲突时保持 journal；重开可成功但所有 canonical API 仍失败关闭。
    if let Err(error) = check(broker, &active) {
        return match error {
            BrokerError::RevisionConflict { .. }
            | BrokerError::UnstableRead(_)
            | BrokerError::CaptureChanged => Ok(()),
            other => Err(other),
        };
    }
    if active.phase != Phase::Completed {
        active.phase = Phase::Installing;
        state.batch = Some(active.clone());
        mutation::save(broker, state)?;
        mutation::fault(broker, 41)?;
        for (index, item) in active.items.iter().enumerate() {
            if active.rollback {
                if !restored(broker, item)? {
                    undo(broker, item).map_err(|_| BrokerError::MutationUnresolved)?;
                }
            } else if require_installed(broker, item).is_err() {
                let sealed = mutation::install::check_before_install(broker, item)
                    .map_err(|_| BrokerError::MutationUnresolved)?;
                let mut installing = item.clone();
                installing.phase = Phase::Installing;
                mutation::install::install(broker, &installing, &sealed)
                    .map_err(|_| BrokerError::MutationUnresolved)?;
            }
            if index == 0 {
                mutation::fault(broker, 42)?;
            }
        }
        // 最后一项完成以后重新验证整批，不能把并发带外修改当作成功。
        for item in &active.items {
            if active.rollback {
                if !restored(broker, item)? {
                    return Err(BrokerError::MutationUnresolved);
                }
            } else {
                require_installed(broker, item).map_err(|_| BrokerError::MutationUnresolved)?;
            }
        }
        active.phase = Phase::Completed;
        state.batch = Some(active.clone());
        mutation::save(broker, state)?;
        mutation::fault(broker, 43)?;
    }
    record::save_terminal(broker, &active)?;
    mutation::fault(broker, 44)?;
    state.committed_through = active.sequence;
    state.batch = None;
    mutation::save(broker, state)?;
    super::barrier::clear_if_idle(broker)
}
fn undo(broker: &FsVaultBroker, item: &Operation) -> Result<(), BrokerError> {
    require_installed(broker, item)?;
    if let Some(bytes) = original(broker, item)? {
        restore_original(broker, item, bytes)?;
    } else {
        let current = crate::blob::current::observe(broker, &item.path)?;
        let (parent, name) = item
            .path
            .as_str()
            .rsplit_once('/')
            .unwrap_or((".", item.path.as_str()));
        let target = directory(&broker.root, parent)?;
        let identity = crate::writer::Identity::directory(&target)?;
        broker.writer.check(&broker.root, &broker.data)?;
        crate::blob::current::check(broker, &item.path, &current)?;
        if crate::writer::Identity::directory(&directory(&broker.root, parent)?)? != identity {
            return Err(BrokerError::RootChanged);
        }
        target
            .remove_file(name)
            .map_err(|e| BrokerError::io("rollback batch create", e))?;
        FsVaultBroker::sync_directory(&target, Path::new("."))?;
    }
    if !restored(broker, item)? {
        return Err(BrokerError::MutationUnresolved);
    }
    Ok(())
}
fn restore_original(
    broker: &FsVaultBroker,
    item: &Operation,
    bytes: Vec<u8>,
) -> Result<(), BrokerError> {
    // stage 先落到既有 staging，再冻结当前父链；在目标目录建临时文件会使自己的 ctime CAS 失效。
    let request = crate::Mutation::replace(
        item.path.clone(),
        bytes,
        item.target_revision,
        format!("batch-rollback:{}:{}", item.sequence, item.path),
    );
    let mut reverse = Operation {
        recording: None,
        sequence: item.sequence,
        key_hash: crate::sha256(request.idempotency_key.as_bytes()),
        fingerprint: FsVaultBroker::fingerprint(&request)?,
        path: item.path.clone(),
        precondition: request.precondition,
        target_revision: item
            .previous_revision
            .ok_or(BrokerError::CorruptWriterState)?,
        previous_revision: Some(item.target_revision),
        stage: mutation::install::prepare(broker, &request, item.sequence)?,
        phase: Phase::Prepared,
        receipt: None,
    };
    let sealed = mutation::install::check_before_install(broker, &reverse)?;
    reverse.phase = Phase::Installing;
    mutation::install::install(broker, &reverse, &sealed)
}
