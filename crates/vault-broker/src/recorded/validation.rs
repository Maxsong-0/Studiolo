//! 请求及终态均绑定不可变描述符；旧格式不伪造元数据。
use super::{MutationMetadata, store};
use crate::{
    BrokerError, FsVaultBroker, Mutation,
    mutation::{
        self,
        record::{Outcome, Terminal},
    },
    records::WriterRecords,
    sha256,
};

pub(crate) fn check_existing(
    broker: &FsVaultBroker,
    request: &Mutation,
    metadata: Option<&MutationMetadata>,
) -> Result<(), BrokerError> {
    let hash = sha256(request.idempotency_key.as_bytes());
    crate::file_operation::guard_legacy_key(broker, &hash)?;
    if let Some(envelope) = store::load_id(broker, &store::id(broker, &hash))? {
        envelope.matches(request, metadata)?;
    }
    if let Some(mutation::replay::Replay::Current(terminal)) =
        mutation::replay::load(broker, &hash, broker.writer.binding.pair_id)?
    {
        if metadata.is_some() && terminal.recording.is_none() {
            return Err(BrokerError::IdempotencyConflict);
        }
        validate_terminal(broker, &terminal)?;
    }
    Ok(())
}

pub(crate) fn validate_terminal(
    records: &dyn WriterRecords,
    terminal: &Terminal,
) -> Result<(), BrokerError> {
    let Some(reference) = &terminal.recording else {
        return Ok(());
    };
    bound(terminal, &store::load(records, reference)?)
}

/// 终态说的就是这条描述符的请求。
pub(crate) fn bound(terminal: &Terminal, envelope: &store::Envelope) -> Result<(), BrokerError> {
    let (path, precondition) = match &terminal.outcome {
        Outcome::Committed { receipt } => (
            &receipt.path,
            receipt
                .previous_revision
                .map(crate::Precondition::Match)
                .unwrap_or(crate::Precondition::MustNotExist),
        ),
        Outcome::Rejected {
            path, precondition, ..
        }
        | Outcome::Cancelled { path, precondition }
        | Outcome::Indeterminate {
            path, precondition, ..
        } => (path, *precondition),
    };
    if path != &envelope.path
        || precondition != envelope.precondition
        || terminal.fingerprint != envelope.fingerprint
        || terminal.expected_revision != envelope.target
    {
        return Err(BrokerError::CorruptWriterState);
    }
    Ok(())
}
