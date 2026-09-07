//! 业务请求只在本模块穿过 CAS、历史、阶段日志与安装的固定顺序。

use super::{
    fault, finish, install, load,
    record::{Operation, Outcome, Phase, Terminal, cas_matches, conflict},
    replay, save, save_terminal,
};
use crate::writer::disk;
use crate::{BrokerError, FsVaultBroker, Mutation, MutationReceipt, Revision, sha256};

pub(super) fn apply(
    broker: &FsVaultBroker,
    mutation: Mutation,
) -> Result<MutationReceipt, BrokerError> {
    if mutation.path.is_protected() {
        return Err(BrokerError::ProtectedPath(mutation.path));
    }
    if mutation.idempotency_key.is_empty() {
        return Err(BrokerError::IdempotencyConflict);
    }
    if mutation.idempotency_key.len() > 4096 || mutation.content.len() > disk::CONTENT_LIMIT {
        return Err(BrokerError::WriterLimitExceeded);
    }
    let mut state = load(broker)?;
    if broker.writer.uncertain() || state.legacy_unresolved {
        return Err(BrokerError::MutationUnresolved);
    }
    if state
        .operation
        .as_ref()
        .is_some_and(|op| op.phase == Phase::Installing)
    {
        return Err(BrokerError::MutationUnresolved);
    }
    finish(broker, &mut state)?;
    let fingerprint = FsVaultBroker::fingerprint(&mutation)?;
    let target = Revision::compute(broker.vault_uuid, &mutation.path, &mutation.content);
    let hash = sha256(mutation.idempotency_key.as_bytes());
    if let Some(op) = &state.operation {
        if op.key_hash != hash {
            return Err(BrokerError::MutationUnresolved);
        }
        if !op.matches(&mutation, &fingerprint, target) {
            return Err(BrokerError::IdempotencyConflict);
        }
        install::verify_stage(broker, op)?;
    } else if let Some(previous) = replay::load(&broker.data, &hash, state.pair_id)? {
        let result = replay::matches(
            previous,
            &fingerprint,
            target,
            &mutation.path,
            mutation.precondition,
        );
        broker.writer.check(&broker.root, &broker.data)?;
        return result;
    }
    let current = match broker.read_checked(&mutation.path, Some(disk::CONTENT_LIMIT)) {
        Ok(object) => Some(object),
        Err(BrokerError::NotFound(_)) => None,
        Err(e) => return Err(e),
    };
    let actual = current.as_ref().map(|object| object.revision);
    if !cas_matches(mutation.precondition, actual) {
        // 明确拒绝也是终态；后来的同内容不能把此键升级成成功。
        reject(broker, &mut state, &mutation, fingerprint, target, actual)?;
        return Err(conflict(mutation.path, mutation.precondition, actual));
    }
    if let Some(object) = &current {
        install::history(broker, object)?;
    }
    if state.operation.is_none() {
        let sequence = state.next_sequence;
        state.next_sequence = sequence
            .checked_add(1)
            .ok_or(BrokerError::WriterLimitExceeded)?;
        let stage = install::prepare(broker, &mutation, sequence)?;
        state.operation = Some(Operation {
            sequence,
            key_hash: hash,
            fingerprint,
            path: mutation.path.clone(),
            precondition: mutation.precondition,
            target_revision: target,
            previous_revision: actual,
            stage,
            phase: Phase::Prepared,
            receipt: None,
        });
        save(broker, &state)?;
        fault(broker, 1)?;
    }
    let op = state
        .operation
        .as_mut()
        .ok_or(BrokerError::CorruptWriterState)?;
    // 安装之前最后一次检查不改变阶段。之后任意不确定错误都保留 Installing。
    if let Err(error) = install::check_before_install(broker, op) {
        if let BrokerError::RevisionConflict { actual, .. } = &error {
            let fingerprint = op.fingerprint.clone();
            reject(broker, &mut state, &mutation, fingerprint, target, *actual)?;
        }
        return Err(error);
    }
    let op = state
        .operation
        .as_mut()
        .ok_or(BrokerError::CorruptWriterState)?;
    op.phase = Phase::Installing;
    save(broker, &state)?;
    fault(broker, 2)?;
    let op = state
        .operation
        .as_ref()
        .ok_or(BrokerError::CorruptWriterState)?;
    if let Err(error) = install::install(broker, op) {
        return Err(match error {
            BrokerError::RevisionConflict { .. } => BrokerError::MutationUnresolved,
            other => other,
        });
    }
    fault(broker, 3)?;
    let op = state
        .operation
        .as_mut()
        .ok_or(BrokerError::CorruptWriterState)?;
    op.phase = Phase::Completed;
    op.receipt = Some(op.receipt());
    let receipt = op.receipt();
    save(broker, &state)?;
    fault(broker, 4)?;
    finish(broker, &mut state)?;
    Ok(receipt)
}

fn reject(
    broker: &FsVaultBroker,
    state: &mut super::State,
    mutation: &Mutation,
    fingerprint: String,
    target: Revision,
    actual: Option<Revision>,
) -> Result<(), BrokerError> {
    let terminal = Terminal {
        format_version: 2,
        pair_id: state.pair_id,
        sequence: None,
        fingerprint,
        expected_revision: target,
        outcome: Outcome::Rejected {
            path: mutation.path.clone(),
            precondition: mutation.precondition,
            actual,
        },
    };
    let hash = sha256(mutation.idempotency_key.as_bytes());
    save_terminal(broker, &hash, &terminal)?;
    fault(broker, 6)?;
    // 复用重开时的取消完成路径，覆盖拒绝已落盘但取消中断的窗口。
    finish(broker, state)
}
