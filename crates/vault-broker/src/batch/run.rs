//! 准备全批后占用既有 writer 槽；不循环调用单文件 mutation。
use super::{BatchPreview, BatchReceipt, BatchRequest, BatchWriter, plan, record, recovery};
use crate::mutation::record::{Operation, Phase};
use crate::{BrokerError, FsVaultBroker, Mutation, Revision, mutation};

pub(super) fn idle(broker: &FsVaultBroker) -> Result<mutation::State, BrokerError> {
    crate::maintenance::ensure_idle(broker)?;
    let state = mutation::load(broker)?;
    if broker.writer.uncertain()
        || state.legacy_unresolved
        || state.operation.is_some()
        || state.file_operation.is_some()
        || state.batch.is_some()
    {
        return Err(BrokerError::MutationUnresolved);
    }
    Ok(state)
}
impl BatchWriter for FsVaultBroker {
    fn preview_batch(&self, request: &BatchRequest) -> Result<BatchPreview, BrokerError> {
        idle(self)?;
        let (preview, _) =
            plan::build(self.vault_uuid, request, |path| self.inspect_existing(path))?;
        crate::memory_write::guard::filesystem(self, &preview.diffs)?;
        Ok(preview)
    }
    fn commit_batch(
        &mut self,
        request: BatchRequest,
        preview: &BatchPreview,
    ) -> Result<BatchReceipt, BrokerError> {
        plan::validate(&request)?;
        let mut state = idle(self)?;
        let request_hash = Self::fingerprint(&request)?;
        let preview_hash = Self::fingerprint(preview)?;
        if let Some(prior) = record::terminal(self, &request.batch_id)? {
            if prior.request_hash != request_hash || prior.preview_hash != preview_hash {
                return Err(BrokerError::IdempotencyConflict);
            }
            return if prior.rollback {
                Err(BrokerError::MutationCancelled)
            } else {
                Ok(prior.receipt())
            };
        }
        let (expected, mutations) = plan::build(self.vault_uuid, &request, |path| {
            self.inspect_existing(path)
        })?;
        if &expected != preview {
            return Err(BrokerError::IdempotencyConflict);
        }
        crate::memory_write::guard::filesystem(self, &expected.diffs)?;
        let sequence = state.next_sequence;
        let mut items = Vec::new();
        for (value, diff) in mutations.iter().zip(&expected.diffs) {
            if let Some(before) = &diff.before {
                mutation::install::history(
                    self,
                    &crate::VaultObject {
                        path: value.path.clone(),
                        content: before.as_bytes().to_vec(),
                        revision: Revision::compute(
                            self.vault_uuid,
                            &value.path,
                            before.as_bytes(),
                        ),
                    },
                )?;
            }
            mutation::install::history(
                self,
                &crate::VaultObject {
                    path: value.path.clone(),
                    content: value.content.clone(),
                    revision: Revision::compute(self.vault_uuid, &value.path, &value.content),
                },
            )?;
            items.push(prepare(self, value, sequence)?);
        }
        // 所有 stage 已完成后，再对全批执行最后一次 CAS 预检。
        for item in &items {
            mutation::install::check_before_install(self, item)?;
        }
        state.format_version = 7;
        state.next_sequence = sequence
            .checked_add(1)
            .ok_or(BrokerError::WriterLimitExceeded)?;
        state.batch = Some(record::Active {
            batch_id: request.batch_id,
            request_hash,
            preview_hash,
            fingerprint: expected.fingerprint,
            sequence,
            rollback: false,
            phase: Phase::Prepared,
            items,
        });
        super::barrier::create(self, &state)?;
        mutation::save(self, &state)?;
        mutation::fault(self, 40)?;
        recovery::resume(self, &mut state)?;
        if state.batch.is_some() {
            return Err(BrokerError::MutationUnresolved);
        }
        record::terminal(self, &expected.batch_id)?
            .map(|v| v.receipt())
            .ok_or(BrokerError::CorruptWriterState)
    }
    fn rollback_batch(&mut self, batch_id: &str) -> Result<BatchReceipt, BrokerError> {
        let mut state = mutation::load(self)?;
        if let Some(mut active) = state.batch.clone() {
            if active.batch_id != batch_id
                || self.writer.uncertain()
                || crate::maintenance::blocked(self)?
            {
                return Err(BrokerError::MutationUnresolved);
            }
            // 原批次尚未公开：以同一事务槽切换为整批回滚，原文和已安装项都先核验。
            active.rollback = true;
            active.phase = Phase::Prepared;
            recovery::check(self, &active)?;
            state.batch = Some(active);
            mutation::save(self, &state)?;
            recovery::resume(self, &mut state)?;
            if state.batch.is_some() {
                return Err(BrokerError::MutationUnresolved);
            }
            return record::terminal(self, batch_id)?
                .map(|v| v.receipt())
                .ok_or(BrokerError::CorruptWriterState);
        }
        let mut state = idle(self)?;
        let mut active =
            record::terminal(self, batch_id)?.ok_or_else(|| plan::invalid("批次不存在"))?;
        if active.rollback {
            return Ok(active.receipt());
        }
        // 全批任何目标被后来编辑，连第一项也不得恢复。
        for item in &active.items {
            recovery::require_installed(self, item)?;
            recovery::original(self, item)?;
        }
        active.rollback = true;
        active.phase = Phase::Prepared;
        active.sequence = state.next_sequence;
        for item in &mut active.items {
            item.sequence = active.sequence;
        }
        state.format_version = 7;
        state.next_sequence = active
            .sequence
            .checked_add(1)
            .ok_or(BrokerError::WriterLimitExceeded)?;
        state.batch = Some(active);
        super::barrier::create(self, &state)?;
        mutation::save(self, &state)?;
        mutation::fault(self, 40)?;
        recovery::resume(self, &mut state)?;
        if state.batch.is_some() {
            return Err(BrokerError::MutationUnresolved);
        }
        record::terminal(self, batch_id)?
            .map(|v| v.receipt())
            .ok_or(BrokerError::CorruptWriterState)
    }
}
fn prepare(
    broker: &FsVaultBroker,
    value: &Mutation,
    sequence: u64,
) -> Result<Operation, BrokerError> {
    Ok(Operation {
        recording: None,
        sequence,
        key_hash: crate::sha256(value.idempotency_key.as_bytes()),
        fingerprint: FsVaultBroker::fingerprint(value)?,
        path: value.path.clone(),
        precondition: value.precondition,
        target_revision: Revision::compute(broker.vault_uuid, &value.path, &value.content),
        previous_revision: match value.precondition {
            crate::Precondition::MustNotExist => None,
            crate::Precondition::Match(r) => Some(r),
        },
        stage: mutation::install::prepare(broker, value, sequence)?,
        phase: Phase::Prepared,
        receipt: None,
    })
}
