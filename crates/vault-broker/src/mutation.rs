//! 一笔固定写入协议：准备可复核，安装未决不可猜测，完成仅补回执。

mod install;
mod record;
mod replay;
mod run;
#[cfg(test)]
mod tests;

use crate::writer::disk;
use crate::{BrokerError, FsVaultBroker, Mutation, MutationReceipt, VaultUuid};
pub(crate) use record::State;
use record::{Outcome, Phase, Terminal};

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
    broker.writer.check(&broker.root, &broker.data)?;
    broker.writer.observe_committed(state.committed_through)?;
    Ok(state)
}

fn save(broker: &FsVaultBroker, state: &State) -> Result<(), BrokerError> {
    state.validate(broker.writer.binding.pair_id)?;
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

fn finish(broker: &FsVaultBroker, state: &mut State) -> Result<(), BrokerError> {
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
            || terminal.expected_revision != op.target_revision
            || path != &op.path
            || *precondition != op.precondition
        {
            return Err(BrokerError::CorruptWriterState);
        }
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
    let terminal = Terminal {
        format_version: 2,
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
    fault(broker, 5)?;
    state.committed_through = op.sequence;
    state.operation = None;
    save(broker, state)
}

fn save_terminal(
    broker: &FsVaultBroker,
    key_hash: &str,
    terminal: &Terminal,
) -> Result<(), BrokerError> {
    terminal.validate(broker.writer.binding.pair_id)?;
    if let Some(existing) = replay::load(&broker.data, key_hash, terminal.pair_id)? {
        match existing {
            replay::Replay::Current(existing) if existing == *terminal => return Ok(()),
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
    result
}

pub(crate) fn apply(
    broker: &FsVaultBroker,
    mutation: Mutation,
) -> Result<MutationReceipt, BrokerError> {
    run::apply(broker, mutation)
}

// 私有故障点只存在于单元测试；没有环境变量、公开接口或生产行为开关。
fn fault(broker: &FsVaultBroker, point: u8) -> Result<(), BrokerError> {
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
