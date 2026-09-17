//! 所有大字节复制/哈希和旧新历史同步都在返回票据前完成；commit 只接受相同证据。
use super::{
    PreparedStream, StreamMutation, request,
    stage::{self, Input},
};
use crate::blob::{current, store};
use crate::mutation::record::{Operation, Outcome, Phase, Terminal, cas_matches, conflict};
use crate::writer::directory;
use crate::{BlobSource, BrokerError, FsVaultBroker, MutationMetadata, StreamLimits, mutation};
use std::sync::Arc;

impl FsVaultBroker {
    /// 普通附件保存先准备；owner 最终复验授权/范围/epoch 后才能 commit。
    pub fn prepare_stream_mutation(
        &mut self,
        request: StreamMutation,
        source: &BlobSource,
        limits: StreamLimits,
        metadata: MutationMetadata,
    ) -> Result<PreparedStream, BrokerError> {
        if !matches!(metadata.kind, crate::MutationKind::Replace) {
            return Err(BrokerError::IdempotencyConflict);
        }
        prepare(self, request, Input::External(source), limits, metadata)
    }
}
pub(super) fn prepare(
    broker: &mut FsVaultBroker,
    request: StreamMutation,
    input: Input<'_>,
    limits: StreamLimits,
    metadata: MutationMetadata,
) -> Result<PreparedStream, BrokerError> {
    request::validate(&request)?;
    crate::file_operation::guard_legacy_key(
        broker,
        &crate::sha256(request.idempotency_key.as_bytes()),
    )?;
    crate::maintenance::ensure_idle(broker)?;
    request::validate(&request)?;
    limits.validate()?;
    if request.content.bytes > limits.effective_limit {
        return Err(BrokerError::WriterLimitExceeded);
    }
    let mut state = mutation::load(broker)?;
    if broker.writer.uncertain() || state.legacy_unresolved || state.operation.is_some() {
        return Err(BrokerError::MutationUnresolved);
    }
    if broker
        .stream_mutation_replay(&request, &metadata)?
        .is_some()
    {
        return Err(BrokerError::IdempotencyConflict);
    }
    crate::recorded::pending::ensure_slot(
        broker,
        &crate::recorded::store::id(broker, &crate::sha256(request.idempotency_key.as_bytes())),
    )?;
    if matches!(metadata.kind, crate::MutationKind::Replace)
        && broker.content_audit_status()?.restore_unconfirmed
    {
        return Err(BrokerError::MutationUnresolved);
    }
    broker.validate_components(&request.path, true)?;
    broker.ensure_parent_dirs(&request.path)?;
    crate::history::preflight(broker, &request.path)?;
    // 先启用新格式，旧程序不可在后续写入中把新描述符语义回写成 v2/v3。
    if state.format_version < 4 {
        state.format_version = state.format_version.max(4);
        mutation::save(broker, &state)?;
    }
    let current = current::observe(broker, &request.path)?;
    let (stage, revision) = stage::create(broker, &request, input, limits, state.next_sequence)?;
    if !cas_matches(request.precondition, current.revision()) {
        let envelope = request::prepare(broker, &request, revision, &metadata)?;
        let terminal = Terminal {
            format_version: 4,
            recording: Some(envelope.reference()?),
            pair_id: state.pair_id,
            sequence: None,
            fingerprint: envelope.fingerprint.clone(),
            expected_revision: revision,
            outcome: Outcome::Rejected {
                path: request.path.clone(),
                precondition: request.precondition,
                actual: current.revision(),
            },
        };
        mutation::save_terminal(broker, &envelope.key_hash, &terminal)?;
        return Err(conflict(
            request.path,
            request.precondition,
            current.revision(),
        ));
    }
    let history = directory(&broker.data, "history")?;
    if let current::Current::Present(proof) = &current {
        let previous = proof.revision.ok_or(BrokerError::CorruptWriterState)?;
        store::copy(
            broker,
            &broker.root,
            &request.path,
            &history,
            &previous.to_string(),
            (&request.path, previous),
            &proof.identity,
        )?;
    }
    let stages = directory(&broker.root, ".workbench/staging")?;
    store::copy(
        broker,
        &stages,
        &store::logical(&stage.name)?,
        &history,
        &revision.to_string(),
        (&request.path, revision),
        &request.content,
    )?;
    current::check(broker, &request.path, &current)?;
    let sealed = store::verify(broker, &stages, &stage.name, &request.path, revision)?;
    if sealed.node.file.identity != stage.identity || sealed.identity != request.content {
        return Err(BrokerError::CaptureChanged);
    }
    let envelope = request::prepare(broker, &request, revision, &metadata)?;
    let operation = Operation {
        recording: Some(envelope.reference()?),
        sequence: state.next_sequence,
        key_hash: envelope.key_hash.clone(),
        fingerprint: envelope.fingerprint.clone(),
        path: request.path,
        precondition: request.precondition,
        target_revision: revision,
        previous_revision: current.revision(),
        stage,
        phase: Phase::Prepared,
        receipt: None,
    };
    // 所有旧/新 L0 及其目录写入完成后才冻结证明，避免自己的保全写入使父链失效。
    let history = crate::blob::install::HistoryPins::capture(broker, &operation)?;
    current::check(broker, &operation.path, &current)?;
    crate::blob::file::check(
        &stages,
        &store::logical(&operation.stage.name)?,
        &sealed.node,
    )?;
    state.next_sequence = state
        .next_sequence
        .checked_add(1)
        .ok_or(BrokerError::WriterLimitExceeded)?;
    state.operation = Some(operation.clone());
    mutation::save(broker, &state)?;
    mutation::fault(broker, 1)?;
    Ok(PreparedStream {
        instance: Arc::clone(&broker.capture_instance),
        reference: envelope.reference()?,
        operation,
        current,
        stage: sealed,
        history,
        intent: crate::content_audit::store::row(&envelope, None)?.event,
    })
}
