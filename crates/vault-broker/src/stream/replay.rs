//! 已提交流式回执只核对固定元数据，不重新读取来源或当前页面。
use super::{
    StreamMutation,
    request::{matches, validate},
};
use crate::{BrokerError, FsVaultBroker, MutationMetadata, mutation, recorded::store, sha256};
impl FsVaultBroker {
    /// 只读重放不依赖来源文件、当前 canonical 或当前协商额度；仍精确绑定 actor/kind。
    pub fn stream_mutation_replay(
        &self,
        request: &StreamMutation,
        metadata: &MutationMetadata,
    ) -> Result<Option<crate::MutationReceipt>, BrokerError> {
        validate(request)?;
        let before = mutation::load(self)?;
        if self.writer.uncertain() || before.legacy_unresolved {
            return Err(BrokerError::MutationUnresolved);
        }
        let hash = sha256(request.idempotency_key.as_bytes());
        crate::file_operation::guard_legacy_key(self, &hash)?;
        let envelope = store::load_id(self, &store::id(self, &hash))?;
        let replay = mutation::replay::load(&self.data, &hash, before.pair_id)?;
        let Some(value) = envelope else {
            return if replay.is_some() {
                Err(BrokerError::IdempotencyConflict)
            } else {
                Ok(None)
            };
        };
        matches(&value, request, metadata)?;
        let result = replay
            .map(|previous| {
                let mutation::replay::Replay::Current(terminal) = &previous else {
                    return Err(BrokerError::IdempotencyConflict);
                };
                if terminal.recording.as_ref() != Some(&value.reference()?) {
                    return Err(BrokerError::CorruptWriterState);
                }
                crate::recorded::validate_terminal(self, terminal)?;
                mutation::replay::matches(
                    previous,
                    &value.fingerprint,
                    value.target,
                    &value.path,
                    value.precondition,
                )
            })
            .transpose();
        self.writer.check(&self.root, &self.data)?;
        if serde_json::to_vec(&before).ok() != serde_json::to_vec(&mutation::load(self)?).ok() {
            return Err(BrokerError::CaptureChanged);
        }
        result
    }
}
