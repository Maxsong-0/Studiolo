//! 实际唯一 Broker 整体进入封存窗口；原租约与全部可变接口一并保留，解冻返还原实例。
pub(crate) mod audit;
mod copy;
mod files;
mod replays;
mod roots;
mod tree;

use crate::{
    BrokerError, FsVaultBroker,
    residue::{DATA_QUARANTINE, VAULT_QUARANTINE},
};
pub use audit::{BrokerCheckpointAudit, BrokerCheckpointCoverage};
use std::{path::Path, time::Instant};
use studiolo_core::checkpoint::{
    CheckpointOwner, OwnerWatermark, SealedOwnerCheckpoint, WalCheckpointProof,
};

/// 冻结的实际 owner；没有第二写者、可克隆句柄或绕过冻结的普通读写接口。
pub struct FrozenBroker {
    broker: FsVaultBroker,
    deadline: Instant,
    verified: Option<(tree::TreeSeal, tree::TreeSeal, BrokerCheckpointCoverage)>,
}
impl FsVaultBroker {
    /// 交出完整实例并保留原租约。构造不丢弃 owner；检查失败仍能经 thaw 返还。
    pub fn freeze_checkpoint(self, deadline: Instant) -> FrozenBroker {
        FrozenBroker {
            broker: self,
            deadline,
            verified: None,
        }
    }
}
impl FrozenBroker {
    /// 只接受实际持久已完成水位；Prepared 可读也不能证明排空。
    pub fn watermark(&self) -> Result<OwnerWatermark, BrokerError> {
        check_deadline(self.deadline)?;
        let state = self.broker.write_status()?;
        if state.unresolved || !crate::content_audit::store::entries(&self.broker)?.is_empty() {
            return Err(BrokerError::MutationUnresolved);
        }
        Ok(OwnerWatermark::Broker {
            vault_uuid: self.broker.vault_uuid,
            pair_id: self.broker.writer.binding.pair_id,
            committed_through: state.committed_through,
            unresolved: false,
        })
    }
    /// 可信组合根须用原 FrozenAudit 完整核对事件、offset 和 hash，不能只接受正整数。
    /// 核对覆盖已清理 outbox 的永久 ACK；失败会撤销先前的导出资格。
    pub fn verify_audit(
        &mut self,
        mut verify: impl FnMut(BrokerCheckpointAudit<'_>, u64, &str) -> Result<(), BrokerError>,
    ) -> Result<BrokerCheckpointCoverage, BrokerError> {
        self.verified = None;
        let watermark = self.watermark()?;
        let before = self.source_seals()?;
        replays::verify(&self.broker, self.deadline)?;
        let coverage = audit::verify(&self.broker, self.deadline, &mut verify)?;
        if before != self.source_seals()? || watermark != self.watermark()? {
            return Err(BrokerError::CaptureChanged);
        }
        self.verified = Some((before.0, before.1, coverage));
        Ok(coverage)
    }
    /// 导出到两个已批准、私有、空且不与源重叠的目录；没有覆写或隐式清理。
    /// 全量物化保留逻辑/物理绑定原字节；恢复只能再走 owner 的新根重绑定协议。
    /// 本票据不替代组合根逐对象 inclusion plan 与最终传输源封存。
    pub fn export(&self, vault: &Path, data: &Path) -> Result<SealedOwnerCheckpoint, BrokerError> {
        let (expected_vault, expected_data, _) = self
            .verified
            .as_ref()
            .ok_or(BrokerError::IncompleteCapture)?;
        let watermark = self.watermark()?;
        if self.source_seals()? != (expected_vault.clone(), expected_data.clone()) {
            return Err(BrokerError::CaptureChanged);
        }
        let target = roots::Targets::new(&self.broker, vault, data)?;
        target.check()?;
        let actual_vault = tree::copy(
            &self.broker.root,
            &target.vault,
            VAULT_QUARANTINE,
            self.deadline,
        )?;
        let actual_data = tree::copy(
            &self.broker.data,
            &target.data,
            DATA_QUARANTINE,
            self.deadline,
        )?;
        if &actual_vault != expected_vault
            || &actual_data != expected_data
            || tree::scan(&target.vault, VAULT_QUARANTINE, self.deadline)? != actual_vault
            || tree::scan(&target.data, DATA_QUARANTINE, self.deadline)? != actual_data
            || self.source_seals()? != (actual_vault, actual_data)
            || self.watermark()? != watermark
        {
            return Err(BrokerError::CaptureChanged);
        }
        target.check()?;
        FsVaultBroker::sync_directory(&target.vault, Path::new("."))?;
        FsVaultBroker::sync_directory(&target.data, Path::new("."))?;
        check_deadline(self.deadline)?;
        Ok(SealedOwnerCheckpoint {
            owner: CheckpointOwner::Broker,
            schema_version: 1,
            wal: WalCheckpointProof {
                busy: 0,
                log_frames: 0,
                checkpointed_frames: 0,
            },
            target_synced: true,
            parent_directory_synced: true,
            reopen_verified: true,
            source_binding_stable: true,
            watermark,
        })
    }
    fn source_seals(&self) -> Result<(tree::TreeSeal, tree::TreeSeal), BrokerError> {
        self.broker
            .writer
            .check(&self.broker.root, &self.broker.data)?;
        let result = (
            tree::scan(&self.broker.root, VAULT_QUARANTINE, self.deadline)?,
            tree::scan(&self.broker.data, DATA_QUARANTINE, self.deadline)?,
        );
        tree::check_pair(&result.0, &result.1)?;
        self.broker
            .writer
            .check(&self.broker.root, &self.broker.data)?;
        check_deadline(self.deadline)?;
        Ok(result)
    }
    /// 无论冻结/核对/导出在哪一步失败，都返还原 owner；不重开、不恢复或清理源。
    #[must_use]
    pub fn thaw(self) -> FsVaultBroker {
        self.broker
    }
}
pub(crate) fn check_deadline(deadline: Instant) -> Result<(), BrokerError> {
    if Instant::now() >= deadline {
        Err(BrokerError::io(
            "Broker 封存截止",
            std::io::Error::new(std::io::ErrorKind::TimedOut, "封存窗口已到期"),
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests;
