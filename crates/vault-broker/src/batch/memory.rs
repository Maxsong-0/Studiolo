//! 内存 adapter 在全批预检之后一次交换对象表；不调用单项 mutate。
use super::*;
use crate::{BrokerError, FsVaultBroker, MemoryVaultBroker, MutationReceipt, Revision};

pub(crate) struct MemoryBatch {
    actor: Option<crate::MutationActor>,
    rollback_actor: Option<crate::MutationActor>,
    request_hash: String,
    preview_hash: String,
    receipt: BatchReceipt,
    originals: Vec<Option<Vec<u8>>>,
}
impl BatchWriter for MemoryVaultBroker {
    fn preview_batch(&self, request: &BatchRequest) -> Result<BatchPreview, BrokerError> {
        let (preview, _) =
            plan::build(self.vault_uuid, request, |path| self.inspect_existing(path))?;
        crate::memory_write::guard::memory(self, &preview.diffs)?;
        Ok(preview)
    }
    fn commit_batch(
        &mut self,
        request: BatchRequest,
        preview: &BatchPreview,
    ) -> Result<BatchReceipt, BrokerError> {
        self.commit_batch_metadata(request, preview, None)
    }
    fn rollback_batch(&mut self, batch_id: &str) -> Result<BatchReceipt, BrokerError> {
        self.rollback_batch_metadata(batch_id, None)
    }
}
impl MemoryVaultBroker {
    /// 固定可信批量主体，保持磁盘adapter同一幂等与历史契约。
    pub fn commit_batch_recorded(
        &mut self,
        request: BatchRequest,
        preview: &BatchPreview,
        metadata: crate::MutationMetadata,
    ) -> Result<BatchReceipt, BrokerError> {
        self.commit_batch_metadata(request, preview, Some(metadata))
    }
    /// 回滚历史记录实际回滚发起者。
    pub fn rollback_batch_recorded(
        &mut self,
        batch_id: &str,
        metadata: crate::MutationMetadata,
    ) -> Result<BatchReceipt, BrokerError> {
        self.rollback_batch_metadata(batch_id, Some(metadata))
    }
    fn commit_batch_metadata(
        &mut self,
        request: BatchRequest,
        preview: &BatchPreview,
        metadata: Option<crate::MutationMetadata>,
    ) -> Result<BatchReceipt, BrokerError> {
        super::run::validate_metadata(metadata.as_ref())?;
        let actor = metadata.map(|m| m.actor);
        plan::validate(&request)?;
        let request_hash = FsVaultBroker::fingerprint(&request)?;
        let preview_hash = FsVaultBroker::fingerprint(preview)?;
        if let Some(prior) = self.batch_replays.get(&request.batch_id) {
            if prior.request_hash != request_hash
                || prior.preview_hash != preview_hash
                || prior.actor != actor
            {
                return Err(BrokerError::IdempotencyConflict);
            }
            return if prior.receipt.rolled_back {
                Err(BrokerError::MutationCancelled)
            } else {
                Ok(prior.receipt.clone())
            };
        }
        let (expected, mutations) = plan::build(self.vault_uuid, &request, |path| {
            self.inspect_existing(path)
        })?;
        if &expected != preview {
            return Err(BrokerError::IdempotencyConflict);
        }
        crate::memory_write::guard::memory(self, &expected.diffs)?;
        for item in &mutations {
            crate::memory_namespace::guard_write(self, &item.path)?;
        }
        let history = self.change_history.prepare(
            mutations
                .iter()
                .zip(&expected.diffs)
                .map(|(m, d)| crate::history::memory::Change {
                    path: m.path.clone(),
                    before: d.before.as_ref().map(|v| {
                        (
                            Revision::compute(self.vault_uuid, &m.path, v.as_bytes()),
                            v.len() as u64,
                        )
                    }),
                    after: Some((
                        Revision::compute(self.vault_uuid, &m.path, &m.content),
                        m.content.len() as u64,
                    )),
                    kind: crate::ChangeKind::Batch,
                })
                .collect(),
            actor.clone(),
        )?;
        let mut next = self.objects.clone();
        let mut receipts = Vec::new();
        let mut originals = Vec::new();
        for item in mutations {
            let original = self.objects.get(&item.path).cloned();
            let previous_revision = original
                .as_ref()
                .map(|bytes| Revision::compute(self.vault_uuid, &item.path, bytes));
            if let (Some(revision), Some(bytes)) = (previous_revision, &original) {
                self.history.insert(revision, bytes.clone());
            }
            let revision = Revision::compute(self.vault_uuid, &item.path, &item.content);
            self.history.insert(revision, item.content.clone());
            next.insert(item.path.clone(), item.content);
            receipts.push(MutationReceipt {
                path: item.path,
                revision,
                previous_revision,
            });
            originals.push(original);
        }
        let receipt = BatchReceipt {
            batch_id: request.batch_id.clone(),
            fingerprint: expected.fingerprint,
            mutations: receipts,
            rolled_back: false,
        };
        self.objects = next;
        self.change_history.commit(history);
        self.batch_replays.insert(
            request.batch_id,
            MemoryBatch {
                actor,
                rollback_actor: None,
                request_hash,
                preview_hash,
                receipt: receipt.clone(),
                originals,
            },
        );
        Ok(receipt)
    }
    fn rollback_batch_metadata(
        &mut self,
        batch_id: &str,
        metadata: Option<crate::MutationMetadata>,
    ) -> Result<BatchReceipt, BrokerError> {
        super::run::validate_metadata(metadata.as_ref())?;
        let actor = metadata.map(|m| m.actor);
        let prior = self
            .batch_replays
            .get(batch_id)
            .ok_or_else(|| plan::invalid("批次不存在"))?;
        if prior.receipt.rolled_back {
            if prior.rollback_actor != actor {
                return Err(BrokerError::IdempotencyConflict);
            }
            return Ok(prior.receipt.clone());
        }
        for item in &prior.receipt.mutations {
            let actual = self.inspect_existing(&item.path)?.map(|o| o.revision);
            if actual != Some(item.revision) {
                return Err(crate::mutation::record::conflict(
                    item.path.clone(),
                    crate::Precondition::Match(item.revision),
                    actual,
                ));
            }
        }
        let history = self.change_history.prepare(
            prior
                .receipt
                .mutations
                .iter()
                .zip(&prior.originals)
                .map(|(m, old)| crate::history::memory::Change {
                    path: m.path.clone(),
                    before: Some((
                        m.revision,
                        self.objects.get(&m.path).map_or(0, |v| v.len() as u64),
                    )),
                    after: old.as_ref().map(|v| {
                        (
                            Revision::compute(self.vault_uuid, &m.path, v),
                            v.len() as u64,
                        )
                    }),
                    kind: crate::ChangeKind::BatchRollback,
                })
                .collect(),
            actor.clone(),
        )?;
        let mut next = self.objects.clone();
        for (item, original) in prior.receipt.mutations.iter().zip(&prior.originals) {
            if let Some(bytes) = original {
                next.insert(item.path.clone(), bytes.clone());
            } else {
                next.remove(&item.path);
            }
        }
        self.objects = next;
        self.change_history.commit(history);
        let prior = self
            .batch_replays
            .get_mut(batch_id)
            .ok_or(BrokerError::CorruptWriterState)?;
        prior.receipt.rolled_back = true;
        prior.rollback_actor = actor;
        Ok(prior.receipt.clone())
    }
}
