//! 业务请求只在本模块穿过 CAS、历史、阶段日志与安装的固定顺序。

use super::{
    fault, finish, install, load,
    record::{Operation, Outcome, Phase, Terminal, cas_matches, conflict},
    replay, save, save_terminal,
};
use crate::{BrokerError, FsVaultBroker, Mutation, MutationReceipt, Revision, sha256};

pub(super) fn apply(
    broker: &FsVaultBroker,
    mutation: Mutation,
    metadata: Option<&crate::MutationMetadata>,
) -> Result<MutationReceipt, BrokerError> {
    super::validate_request(&mutation)?;
    crate::file_operation::guard_legacy_key(broker, &sha256(mutation.idempotency_key.as_bytes()))?;
    crate::maintenance::ensure_idle(broker)?;
    super::validate_request(&mutation)?;
    crate::recorded::check_existing(broker, &mutation, metadata)?;
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
        if let Some(reference) = &op.recording {
            crate::recorded::store::load(broker, reference)?.matches(&mutation, metadata)?;
        } else if metadata.is_some() {
            return Err(BrokerError::MutationUnresolved);
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
    // 原终态只重放；新的安装必须等待其他恢复闭合，原 Restore 仍沿同键续办。
    crate::recorded::pending::ensure_slot(broker, &crate::recorded::store::id(broker, &hash))?;
    if let Some(existing) =
        crate::recorded::store::load_id(broker, &crate::recorded::store::id(broker, &hash))?
        && matches!(existing.kind, Some(crate::MutationKind::Restore { .. }))
        && (!metadata
            .is_some_and(|value| matches!(value.kind, crate::MutationKind::Restore { .. }))
            || !crate::content_audit::store::acknowledged(
                broker,
                &existing,
                &crate::content_audit::store::row(&existing, None)?,
            )?)
    {
        return Err(BrokerError::MutationUnresolved);
    }
    let current = crate::blob::current::observe(broker, &mutation.path)?;
    let actual = current.revision();
    if !cas_matches(mutation.precondition, actual) {
        // 明确拒绝也是终态；后来的同内容不能把此键升级成成功。
        reject(
            broker,
            &mut state,
            &mutation,
            fingerprint,
            target,
            actual,
            metadata,
        )?;
        return Err(conflict(mutation.path, mutation.precondition, actual));
    }
    // 旧 Vec 入口也可能替换 bridge 已摄入的大旧对象；保全前先启用相应索引语义。
    if matches!(&current, crate::blob::current::Current::Present(proof)
        if proof.identity.bytes > crate::writer::disk::CONTENT_LIMIT as u64)
        && state.format_version < 4
    {
        state.format_version = state.format_version.max(4);
        save(broker, &state)?;
    }
    let envelope = crate::recorded::store::prepare(broker, &mutation, metadata)?;
    crate::history::preflight(broker, &mutation.path)?;
    if let crate::blob::current::Current::Present(object) = &current {
        let previous = object.revision.ok_or(BrokerError::CorruptWriterState)?;
        let history = crate::writer::directory(&broker.data, "history")?;
        crate::blob::store::copy(
            broker,
            &broker.root,
            &mutation.path,
            &history,
            &previous.to_string(),
            (&mutation.path, previous),
            &object.identity,
        )?;
    }
    install::history(
        broker,
        &crate::VaultObject {
            path: mutation.path.clone(),
            revision: target,
            content: mutation.content.clone(),
        },
    )?;
    if state.operation.is_none() {
        let sequence = state.next_sequence;
        state.next_sequence = sequence
            .checked_add(1)
            .ok_or(BrokerError::WriterLimitExceeded)?;
        let stage = install::prepare(broker, &mutation, sequence)?;
        state.format_version =
            state
                .format_version
                .max(if state.maintenance_enabled { 3 } else { 2 });
        state.operation = Some(Operation {
            recording: Some(envelope.reference()?),
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
    let sealed = match install::check_before_install(broker, op) {
        Ok(sealed) => sealed,
        Err(error) => {
            if let BrokerError::RevisionConflict { actual, .. } = &error {
                let fingerprint = op.fingerprint.clone();
                reject(
                    broker,
                    &mut state,
                    &mutation,
                    fingerprint,
                    target,
                    *actual,
                    metadata,
                )?;
            }
            return Err(error);
        }
    };
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
    // Installing 已持久化，安装中的任何错误都不能证明 canonical 未改变。
    // 保留原阶段与磁盘证据；具体错误不得被调用方误译为可安全重试的普通拒绝。
    install::install(broker, op, &sealed).map_err(|_| BrokerError::MutationUnresolved)?;
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
    metadata: Option<&crate::MutationMetadata>,
) -> Result<(), BrokerError> {
    let recording = match &state.operation {
        Some(op) => op.recording.clone(),
        None => Some(crate::recorded::store::prepare(broker, mutation, metadata)?.reference()?),
    };
    let terminal = Terminal {
        format_version: if recording.is_some() { 4 } else { 2 },
        recording,
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
