//! 一笔固定写入协议：准备可复核，安装未决不可猜测，完成仅补回执。

pub(crate) mod install;
pub(crate) mod record;
pub(crate) mod replay;
mod run;
#[cfg(test)]
mod tests;

use crate::writer::disk;
use crate::{BrokerError, FsVaultBroker, Mutation, MutationReceipt, VaultUuid};
pub(crate) use record::State;
use record::{Outcome, Phase, Terminal};

pub(crate) fn validate_request(request: &Mutation) -> Result<(), BrokerError> {
    if request.path.is_protected() {
        return Err(BrokerError::ProtectedPath(request.path.clone()));
    }
    if request.idempotency_key.is_empty() {
        return Err(BrokerError::IdempotencyConflict);
    }
    if request.idempotency_key.len() > 4096 || request.content.len() > disk::CONTENT_LIMIT {
        return Err(BrokerError::WriterLimitExceeded);
    }
    Ok(())
}

pub(crate) fn validate_replays(
    data: &cap_std::fs::Dir,
    pair: Option<VaultUuid>,
) -> Result<bool, BrokerError> {
    replay::scan(data, pair).map(|v| v.0)
}

pub(crate) fn load(broker: &FsVaultBroker) -> Result<State, BrokerError> {
    broker.writer.check(&broker.root, &broker.data)?;
    let bytes = disk::read(&broker.data, "writer-state.json", disk::RECORD_LIMIT)?
        .ok_or(BrokerError::CorruptWriterState)?;
    let state: State = disk::decode(&bytes)?;
    state.validate(broker.writer.binding.pair_id)?;
    crate::file_operation::store::validate_state(broker, &state)?;
    if let Some(op) = &state.operation
        && let Some(reference) = &op.recording
    {
        let envelope = crate::recorded::store::load(broker, reference)?;
        if (envelope.version == 2 && state.format_version < 4)
            || envelope.path != op.path
            || envelope.precondition != op.precondition
            || envelope.target != op.target_revision
            || envelope.key_hash != op.key_hash
            || envelope.fingerprint != op.fingerprint
            || envelope.sha256 != op.stage.digest
            || envelope.bytes != op.stage.size
        {
            return Err(BrokerError::CorruptWriterState);
        }
    }
    broker.writer.check(&broker.root, &broker.data)?;
    broker.writer.observe_committed(state.committed_through)?;
    Ok(state)
}

pub(crate) fn save(broker: &FsVaultBroker, state: &State) -> Result<(), BrokerError> {
    state.validate(broker.writer.binding.pair_id)?;
    crate::file_operation::store::validate_state(broker, state)?;
    let result = disk::save(&broker.data, "writer-state.json", state, || {
        broker.writer.check(&broker.root, &broker.data)
    })
    .and_then(|()| broker.writer.check(&broker.root, &broker.data))
    .and_then(|()| broker.writer.observe_committed(state.committed_through));
    if result.is_err() {
        broker.writer.poison();
    }
    result
}

pub(crate) fn reopen(broker: &FsVaultBroker) -> Result<(), BrokerError> {
    let mut state = load(broker)?;
    finish(broker, &mut state)
}

pub(crate) fn finish(broker: &FsVaultBroker, state: &mut State) -> Result<(), BrokerError> {
    if state.file_operation.is_some() {
        return crate::file_operation::finish::resume(broker, state);
    }
    if crate::recovery::finish(broker, state)? {
        return Ok(());
    }
    if let Some(op) = state
        .operation
        .as_ref()
        .filter(|op| op.phase == Phase::Prepared)
        && let Some(replay) = replay::load(&broker.data, &op.key_hash, state.pair_id)?
    {
        let replay::Replay::Current(terminal) = replay else {
            return Err(BrokerError::CorruptWriterState);
        };
        let Outcome::Rejected {
            path, precondition, ..
        } = &terminal.outcome
        else {
            return Err(BrokerError::CorruptWriterState);
        };
        if terminal.fingerprint != op.fingerprint
            || terminal.recording != op.recording
            || terminal.expected_revision != op.target_revision
            || path != &op.path
            || *precondition != op.precondition
        {
            return Err(BrokerError::CorruptWriterState);
        }
        crate::content_audit::stage_outcome(broker, &terminal)?;
        // 拒绝已持久化而取消日志尚未清除：只能完成取消，不能再次尝试安装。
        install::discard_prepared(broker, op)?;
        state.operation = None;
        return save(broker, state);
    }
    let Some(op) = state
        .operation
        .as_ref()
        .filter(|op| op.phase == Phase::Completed)
    else {
        return Ok(());
    };
    crate::history::complete(broker, op)?;
    let terminal = Terminal {
        recording: op.recording.clone(),
        format_version: if op.recording.is_some() { 4 } else { 2 },
        pair_id: state.pair_id,
        sequence: Some(op.sequence),
        fingerprint: op.fingerprint.clone(),
        expected_revision: op.target_revision,
        outcome: Outcome::Committed {
            receipt: op.receipt(),
        },
    };
    match replay::load(&broker.data, &op.key_hash, state.pair_id)? {
        Some(replay::Replay::Current(existing)) => {
            if existing != terminal {
                return Err(BrokerError::CorruptWriterState);
            }
        }
        Some(_) => return Err(BrokerError::CorruptWriterState),
        None => save_terminal(broker, &op.key_hash, &terminal)?,
    }
    crate::content_audit::stage_outcome(broker, &terminal)?;
    fault(broker, 5)?;
    state.committed_through = op.sequence;
    state.operation = None;
    save(broker, state)
}

pub(crate) fn save_terminal(
    broker: &FsVaultBroker,
    key_hash: &str,
    terminal: &Terminal,
) -> Result<(), BrokerError> {
    terminal.validate(broker.writer.binding.pair_id)?;
    crate::recorded::validate_terminal(broker, terminal)?;
    if let Some(existing) = replay::load(&broker.data, key_hash, terminal.pair_id)? {
        match existing {
            replay::Replay::Current(existing) if existing == *terminal => {
                return crate::content_audit::stage_outcome(broker, terminal);
            }
            _ => return Err(BrokerError::CorruptWriterState),
        }
    }
    let result = disk::save(
        &broker.data,
        &format!("idempotency/{key_hash}.json"),
        terminal,
        || broker.writer.check(&broker.root, &broker.data),
    )
    .and_then(|()| broker.writer.check(&broker.root, &broker.data));
    if result.is_err() {
        broker.writer.poison();
    }
    result?;
    crate::content_audit::stage_outcome(broker, terminal)
}

pub(crate) fn apply(
    broker: &FsVaultBroker,
    mutation: Mutation,
) -> Result<MutationReceipt, BrokerError> {
    run::apply(broker, mutation, None)
}

pub(crate) fn apply_recorded(
    broker: &FsVaultBroker,
    mutation: Mutation,
    metadata: &crate::MutationMetadata,
) -> Result<MutationReceipt, BrokerError> {
    run::apply(broker, mutation, Some(metadata))
}

// 私有故障点只存在于单元测试；没有环境变量、公开接口或生产行为开关。
pub(crate) fn fault(broker: &FsVaultBroker, point: u8) -> Result<(), BrokerError> {
    #[cfg(test)]
    if broker
        .writer
        .fault
        .compare_exchange(
            point,
            0,
            std::sync::atomic::Ordering::AcqRel,
            std::sync::atomic::Ordering::Acquire,
        )
        .is_ok()
    {
        return Err(BrokerError::io(
            "injected writer interruption",
            std::io::Error::other("interrupted"),
        ));
    }
    #[cfg(not(test))]
    let _ = (broker, point);
    Ok(())
}
