//! 恢复只能从同路径仍在可见 L0 索引内的对象准备，不能以任意输入冒充原历史。
use super::{PreparedStream, StreamMutation, prepare, stage::Input};
use crate::{
    BrokerError, FsVaultBroker, MutationActor, MutationKind, MutationMetadata, Precondition,
    Revision, StreamLimits, VaultPath, writer::directory,
};
impl FsVaultBroker {
    /// 来源 revision 清理后的原成功应先通过 restore_replay 取得；本方法只准备新恢复。
    pub fn prepare_stream_restore(
        &mut self,
        path: VaultPath,
        expected: Revision,
        source_revision: Revision,
        key: String,
        actor: MutationActor,
        limits: StreamLimits,
    ) -> Result<PreparedStream, BrokerError> {
        if key.is_empty() || key.len() > 4096 {
            return Err(BrokerError::IdempotencyConflict);
        }
        crate::file_operation::guard_legacy_key(self, &crate::sha256(key.as_bytes()))?;
        crate::maintenance::ensure_idle(self)?;
        let metadata =
            MutationMetadata::new(actor.clone(), MutationKind::Restore { source_revision })?;
        if self
            .restore_replay(&path, expected, source_revision, &key, &actor)?
            .is_some()
        {
            return Err(BrokerError::IdempotencyConflict);
        }
        let content = crate::history::blob::selected(self, &path, source_revision)?;
        let input = Input::History {
            root: directory(&self.data, "history")?,
            name: crate::blob::store::logical(&source_revision.to_string())?,
            revision: source_revision,
        };
        prepare::prepare(
            self,
            StreamMutation {
                path,
                precondition: Precondition::Match(expected),
                idempotency_key: key,
                content,
            },
            input,
            limits,
            metadata,
        )
    }
}
