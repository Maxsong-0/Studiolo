//! 历史条目保留成功阶段/回执和不可变来源的绑定，不能借合法 JSON 重写作者。

use super::catalog::Item;
use crate::{
    BrokerError, FsVaultBroker, VaultPath,
    mutation::{
        self,
        record::{Outcome, Phase},
    },
    recorded::store,
    sha256,
};

pub(super) fn validate(
    broker: &FsVaultBroker,
    path: &VaultPath,
    item: &Item,
) -> Result<(), BrokerError> {
    if item.file_operation_id.is_some() {
        return super::file_operation::validate(broker, path, item);
    }
    let entry = &item.entry;
    let Some(reference) = &item.recording else {
        return if item.sequence == 0
            && entry.author.is_none()
            && entry.authored_at_ms.is_none()
            && entry.archived_by.is_none()
            && entry.archived_at_ms.is_none()
        {
            Ok(())
        } else {
            Err(BrokerError::CorruptWriterState)
        };
    };
    let envelope = store::load(broker, reference)?;
    if item.sequence == 0 || &envelope.path != path || entry.archived_by != envelope.actor {
        return Err(BrokerError::CorruptWriterState);
    }
    if entry.id == sha256(format!("{}:new", envelope.id).as_bytes()) {
        if entry.revision != envelope.target
            || entry.bytes != envelope.bytes
            || entry.author != envelope.actor
            || entry.authored_at_ms != envelope.requested_at_ms
        {
            return Err(BrokerError::CorruptWriterState);
        }
    } else if entry.id == sha256(format!("{}:previous", envelope.id).as_bytes()) {
        if envelope.precondition != crate::Precondition::Match(entry.revision)
            || entry.author.is_some()
            || entry.authored_at_ms.is_some()
        {
            return Err(BrokerError::CorruptWriterState);
        }
    } else {
        return Err(BrokerError::CorruptWriterState);
    }
    if let Some(replay) =
        mutation::replay::load(&broker.data, &envelope.key_hash, envelope.pair_id)?
    {
        let mutation::replay::Replay::Current(terminal) = replay else {
            return Err(BrokerError::CorruptWriterState);
        };
        crate::recorded::validate_terminal(broker, &terminal)?;
        if terminal.recording.as_ref() != Some(reference)
            || terminal.sequence != Some(item.sequence)
            || !matches!(terminal.outcome, Outcome::Committed { .. })
        {
            return Err(BrokerError::CorruptWriterState);
        }
    } else {
        // 唯一允许尚无终态的条目处在 Completed：索引必须先可靠保存，水位随后才能公开。
        let state = mutation::load(broker)?;
        if !state.operation.as_ref().is_some_and(|op| {
            op.phase == Phase::Completed
                && op.sequence == item.sequence
                && op.recording.as_ref() == Some(reference)
        }) {
            return Err(BrokerError::CorruptWriterState);
        }
    }
    Ok(())
}
