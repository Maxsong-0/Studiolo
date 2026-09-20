//! 内存 adapter 在全批预检之后一次交换对象表；不调用单项 mutate。
use super::*;
use crate::{BrokerError, FsVaultBroker, MemoryVaultBroker, MutationReceipt, Revision};

pub(crate) struct MemoryBatch {
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
        plan::validate(&request)?;
        let request_hash = FsVaultBroker::fingerprint(&request)?;
        let preview_hash = FsVaultBroker::fingerprint(preview)?;
        if let Some(prior) = self.batch_replays.get(&request.batch_id) {
            if prior.request_hash != request_hash || prior.preview_hash != preview_hash {
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
        self.batch_replays.insert(
            request.batch_id,
            MemoryBatch {
                request_hash,
                preview_hash,
                receipt: receipt.clone(),
                originals,
            },
        );
        Ok(receipt)
    }
    fn rollback_batch(&mut self, batch_id: &str) -> Result<BatchReceipt, BrokerError> {
        let prior = self
            .batch_replays
            .get(batch_id)
            .ok_or_else(|| plan::invalid("批次不存在"))?;
        if prior.receipt.rolled_back {
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
        let mut next = self.objects.clone();
        for (item, original) in prior.receipt.mutations.iter().zip(&prior.originals) {
            if let Some(bytes) = original {
                next.insert(item.path.clone(), bytes.clone());
            } else {
                next.remove(&item.path);
            }
        }
        self.objects = next;
        let prior = self
            .batch_replays
            .get_mut(batch_id)
            .ok_or(BrokerError::CorruptWriterState)?;
        prior.receipt.rolled_back = true;
        Ok(prior.receipt.clone())
    }
}
