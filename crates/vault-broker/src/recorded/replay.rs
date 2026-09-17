//! 恢复重放不依赖已清理的历史正文，也不读取当前 canonical。
use super::{MutationActor, MutationKind, MutationMetadata, store};
use crate::{
    BrokerError, FsVaultBroker, MutationReceipt, Precondition, Revision, VaultPath, mutation,
    sha256,
};
impl FsVaultBroker {
    /// 历史正文被清理后仍核对原恢复描述符并返回永久终态，不读当前文件或重新安装。
    pub fn restore_replay(
        &self,
        path: &VaultPath,
        expected: Revision,
        source_revision: Revision,
        key: &str,
        actor: &MutationActor,
    ) -> Result<Option<MutationReceipt>, BrokerError> {
        if path.is_protected() {
            return Err(BrokerError::ProtectedPath(path.clone()));
        }
        if key.is_empty() || key.len() > 4096 {
            return Err(BrokerError::IdempotencyConflict);
        }
        MutationMetadata::new(actor.clone(), MutationKind::Restore { source_revision })?;
        let state = mutation::load(self)?;
        if self.writer.uncertain() {
            return Err(BrokerError::MutationUnresolved);
        }
        let hash = sha256(key.as_bytes());
        crate::file_operation::guard_legacy_key(self, &hash)?;
        let envelope = store::load_id(self, &store::id(self, &hash))?;
        let replay = mutation::replay::load(&self.data, &hash, state.pair_id)?;
        let Some(envelope) = envelope else {
            return if replay.is_some() {
                Err(BrokerError::IdempotencyConflict)
            } else {
                Ok(None)
            };
        };
        if &envelope.path != path
            || envelope.precondition != Precondition::Match(expected)
            || envelope.target != source_revision
            || envelope.actor.as_ref() != Some(actor)
            || envelope.kind != Some(MutationKind::Restore { source_revision })
        {
            return Err(BrokerError::IdempotencyConflict);
        }
        let result = replay
            .map(|previous| {
                let mutation::replay::Replay::Current(terminal) = &previous else {
                    return Err(BrokerError::CorruptWriterState);
                };
                if terminal.recording.as_ref() != Some(&envelope.reference()?) {
                    return Err(BrokerError::CorruptWriterState);
                }
                super::validate_terminal(self, terminal)?;
                mutation::replay::matches(
                    previous,
                    &envelope.fingerprint,
                    source_revision,
                    path,
                    Precondition::Match(expected),
                )
            })
            .transpose();
        self.writer.check(&self.root, &self.data)?;
        let after = mutation::load(self)?;
        if serde_json::to_vec(&state).ok() != serde_json::to_vec(&after).ok() {
            return Err(BrokerError::CaptureChanged);
        }
        result
    }
}
