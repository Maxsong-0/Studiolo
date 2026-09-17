//! 两个观察、两份路径绑定 L0 和一个 stage 在同一个 Preparing 槽内完成。
use super::{frozen::Frozen, observation::Observation, record::Active, rename_model::Progress, *};
use crate::{BrokerError, FsVaultBroker, mutation};

pub(crate) fn prepare(
    broker: &FsVaultBroker,
    mut state: mutation::State,
    frozen: Frozen,
) -> Result<FileOperationTicket, BrokerError> {
    let FileOperationKind::Rename { expected, to } = &frozen.kind else {
        return Err(BrokerError::CorruptWriterState);
    };
    // 通用 Missing 也可表示祖先目录缺失；Rename 只允许已有父目录，先完整核验双方父链。
    crate::blob::file::parents(&broker.root, &frozen.path)?;
    crate::blob::file::parents(&broker.root, to)?;
    let before = Observation::observe(broker, &frozen.path)?;
    let target = Observation::observe(broker, to)?;
    rename_capacity::layout(&frozen)?;
    let rejected = before.proof().is_none_or(|p| p.revision != Some(*expected))
        || !matches!(target, Observation::Missing { .. });
    let body = if rejected {
        0
    } else {
        before
            .proof()
            .ok_or(BrokerError::CorruptWriterState)?
            .identity
            .bytes
    };
    crate::history::store::open(broker, true)?;
    crate::history::preflight(broker, &frozen.path)?;
    crate::history::preflight(broker, to)?;
    let reserved_bytes = rename_capacity::reserve(broker, body, !rejected)?;
    before.check(broker, &frozen.path)?;
    target.check(broker, to)?;
    let progress = Progress {
        before: target,
        history: None,
        target_revision: None,
        executed: None,
        source_check: None,
        target_effect_at_ms: None,
        install_id: rename_model::step(&frozen.operation_id, "target-install"),
        detach_id: rename_model::step(&frozen.operation_id, "source-detach"),
    };
    let active = Active {
        frozen,
        sequence: state.next_sequence,
        reserved_bytes,
        phase: FileOperationPhase::Preparing,
        before,
        rejected,
        history: None,
        stage: None,
        sealed_stage: None,
        executed: None,
        quarantine: None,
        effect_at_ms: None,
        terminal: None,
        rename: Some(progress),
    };
    state.next_sequence = state
        .next_sequence
        .checked_add(1)
        .ok_or(BrokerError::WriterLimitExceeded)?;
    state.file_operation = Some(active);
    mutation::save(broker, &state)?;
    mutation::fault(broker, 70)?;
    let active = state
        .file_operation
        .as_mut()
        .ok_or(BrokerError::CorruptWriterState)?;
    request::ensure(broker, active)?;
    if !rejected {
        preserve(broker, active)?;
    }
    active.phase = FileOperationPhase::Prepared;
    mutation::save(broker, &state)?;
    mutation::fault(broker, 71)?;
    ticket::from_active(
        broker,
        state
            .file_operation
            .as_ref()
            .ok_or(BrokerError::CorruptWriterState)?,
    )
}
pub(crate) use super::rename_preserve::{check, preserve};
