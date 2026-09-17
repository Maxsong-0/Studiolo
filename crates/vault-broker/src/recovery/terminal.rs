//! 已持久协调终态只完成清理，永不从当前内容补成成功。

use super::{evidence, run};
use crate::mutation::{
    self,
    record::{Outcome, Phase, State},
};
use crate::{BrokerError, FsVaultBroker};

pub(crate) fn finish(broker: &FsVaultBroker, state: &mut State) -> Result<bool, BrokerError> {
    let Some(op) = state.operation.as_ref() else {
        return Ok(false);
    };
    let Some(replay) = mutation::replay::load(&broker.data, &op.key_hash, state.pair_id)? else {
        return Ok(false);
    };
    let mutation::replay::Replay::Current(terminal) = replay else {
        return Ok(false);
    };
    let (path, precondition, streaming_evidence) = match &terminal.outcome {
        Outcome::Cancelled { path, precondition } if op.phase == Phase::Prepared => {
            (path, precondition, false)
        }
        Outcome::Indeterminate {
            path,
            precondition,
            evidence_id,
            current_revision,
        } if op.phase == Phase::Installing => {
            let version = evidence::verify_version(broker, evidence_id, op, *current_revision)?;
            (path, precondition, version == 2)
        }
        Outcome::Cancelled { .. } | Outcome::Indeterminate { .. } => {
            return Err(BrokerError::CorruptWriterState);
        }
        _ => return Ok(false),
    };
    if path != &op.path
        || precondition != &op.precondition
        || terminal.recording != op.recording
        || terminal.fingerprint != op.fingerprint
        || terminal.expected_revision != op.target_revision
    {
        return Err(BrokerError::CorruptWriterState);
    }
    crate::content_audit::stage_outcome(broker, &terminal)?;
    run::discard_optional_stage(broker, op)?;
    mutation::fault(broker, 9)?;
    // 只有已持久的精确终态与完整 v2 证据才能升级；与清 pending 同次保存。
    // 不在 preserve 期间改写日志，避免自己造成原 review/journal 漂移。
    if streaming_evidence {
        state.format_version = state.format_version.max(4);
    }
    state.operation = None;
    mutation::save(broker, state)?;
    Ok(true)
}
