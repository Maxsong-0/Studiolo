//! 完整 canonical 本地范围观察；只产字节事实，不把本地完整冒充远端同步完整。
pub(crate) mod kernel;
mod metadata;
mod model;
mod paths;
mod selection;
pub(crate) mod state;
pub use model::{
    CanonicalSyncFile, CanonicalSyncObservation, CanonicalSyncPresence, CanonicalSyncSurvey,
    SyncReadRule, SyncReadSelection,
};

use crate::{BrokerError, FsVaultBroker, VaultPath, capture};
use model::Gate;
use std::{collections::BTreeMap, sync::Arc};

#[cfg(test)]
#[path = "../tests/sync_observation_private/mod.rs"]
pub(crate) mod tests;

impl FsVaultBroker {
    /// 只读检查完整目录锚点及父链，返回缺失的锚点；不读取正文或改变提交水位。
    /// selection 是可信装配传入的数据，不是写权限；本实例仍须独占 canonical 写租约。
    pub fn inspect_sync_anchors(
        &self,
        selection: &SyncReadSelection,
    ) -> Result<Vec<VaultPath>, BrokerError> {
        let committed = self.sync_anchors_ready()?;
        selection.validate(&[])?;
        let mut missing = Vec::new();
        for rule in &selection.folders {
            if self.sync_anchor_directory(&rule.path, false)? {
                missing.push(rule.path.clone());
            }
            if self.sync_anchors_ready()? != committed {
                return Err(BrokerError::CaptureChanged);
            }
        }
        if self.sync_anchors_ready()? != committed {
            return Err(BrokerError::CaptureChanged);
        }
        Ok(missing)
    }

    /// 显式、幂等准备 folders 的空目录锚点，不处理 files、不写正文或提交记录。
    /// 全部锚点预检通过后才创建；返回本次新建的锚点，不包含中间父目录。
    /// I/O 或并发变化导致失败时可能已创建部分目录；错误不承诺回滚，重新预检后可重试。
    pub fn prepare_sync_anchors(
        &mut self,
        selection: &SyncReadSelection,
    ) -> Result<Vec<VaultPath>, BrokerError> {
        let committed = self.sync_anchors_ready()?;
        self.inspect_sync_anchors(selection)?;
        let mut created = Vec::new();
        for rule in &selection.folders {
            if self.sync_anchors_ready()? != committed {
                return Err(BrokerError::CaptureChanged);
            }
            if self.sync_anchor_directory(&rule.path, true)? {
                created.push(rule.path.clone());
            }
        }
        if !self.inspect_sync_anchors(selection)?.is_empty()
            || self.sync_anchors_ready()? != committed
        {
            return Err(BrokerError::CaptureChanged);
        }
        Ok(created)
    }

    pub(crate) fn sync_anchors_ready(&self) -> Result<u64, BrokerError> {
        self.writer.check(&self.root, &self.data)?;
        let committed = capture::ready(self)?;
        if self.write_status()?.unresolved {
            return Err(BrokerError::MutationUnresolved);
        }
        Ok(committed)
    }

    /// 兼容旧一次式 API；两阶段之间不开放额外能力，任意失败均不返回部分结果。
    /// selection/required 是数据而不是权限；完整结果也不是带外写入下的原子快照。
    pub fn capture_sync_observation(
        &mut self,
        selection: &SyncReadSelection,
        required: &[VaultPath],
    ) -> Result<CanonicalSyncObservation, BrokerError> {
        let survey = self.begin_sync_observation(selection, required)?;
        self.finish_sync_observation(survey, &[])
    }

    /// 只完整观察元数据，保留原选择、全部名字与固定父链，尚未读取正文。
    /// 调用者用 paths() 汇合两侧和基线的完整 U；本阶段不声称 Mirror 已静止。
    pub fn begin_sync_observation(
        &mut self,
        selection: &SyncReadSelection,
        required: &[VaultPath],
    ) -> Result<CanonicalSyncSurvey, BrokerError> {
        let gate = Gate {
            instance: Arc::clone(&self.capture_instance),
            pair: self.writer.binding.pair_id,
            vault: self.vault_uuid,
            committed: capture::ready(self)?,
        };
        let survey = kernel::begin(&self.root, false, selection, required)?;
        self.sync_gate(&gate)?;
        Ok(CanonicalSyncSurvey { gate, survey })
    }

    /// 消费同实例调查，只允许增补原范围内路径；不重新枚举或再次完整读正文。
    /// additional_required 自身重复拒绝，与原已知集合重合允许；被排除项整批拒绝。
    pub fn finish_sync_observation(
        &mut self,
        survey: CanonicalSyncSurvey,
        additional_required: &[VaultPath],
    ) -> Result<CanonicalSyncObservation, BrokerError> {
        self.sync_gate(&survey.gate)?;
        let prepared = kernel::finish(&self.root, false, survey.survey, additional_required)?;
        let mut result = CanonicalSyncObservation {
            gate: survey.gate,
            entries: BTreeMap::new(),
            proofs: prepared.proofs,
        };
        for (path, candidate) in prepared.candidates {
            self.sync_gate(&result.gate)?;
            let (fact, proof) = kernel::read(
                &self.root,
                false,
                &path,
                candidate,
                &prepared.selection,
                Some(self.vault_uuid),
            )?;
            let presence = match fact {
                Some(fact) => CanonicalSyncPresence::Present(CanonicalSyncFile {
                    identity: fact.identity,
                    revision: fact.revision.ok_or(BrokerError::CaptureChanged)?,
                    utf8_valid: fact.utf8_valid,
                }),
                None => CanonicalSyncPresence::Missing,
            };
            result.entries.insert(path.clone(), presence);
            result.proofs.files.insert(path, proof);
        }
        #[cfg(test)]
        tests::hook(2);
        self.revalidate_sync_observation(&result)?;
        Ok(result)
    }

    /// 不重读/重新哈希正文；重验固定根/父链、对象、实例、配对、水位和读门禁。
    /// 必须由同一 owner 串行化本方法与写入，不能用旧结果给新实例授权。
    pub fn revalidate_sync_observation(
        &self,
        observation: &CanonicalSyncObservation,
    ) -> Result<(), BrokerError> {
        self.sync_gate(&observation.gate)?;
        kernel::revalidate(&self.root, false, &observation.proofs)?;
        self.sync_gate(&observation.gate)
    }
    fn sync_gate(&self, gate: &Gate) -> Result<(), BrokerError> {
        let committed = capture::ready(self)?;
        if !Arc::ptr_eq(&self.capture_instance, &gate.instance)
            || gate.pair != self.writer.binding.pair_id
            || gate.vault != self.vault_uuid
            || committed != gate.committed
        {
            return Err(BrokerError::CaptureChanged);
        }
        Ok(())
    }
}

fn invalid() -> BrokerError {
    BrokerError::InvalidArtifact("invalid canonical sync observation selection".to_owned())
}
