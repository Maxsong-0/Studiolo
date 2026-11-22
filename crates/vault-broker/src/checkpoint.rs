//! 实际唯一 Broker 整体进入封存窗口；原租约与全部可变接口一并保留，解冻返还原实例。
//! 窗口里只核回执、定格两棵树（打开文件留句柄，不读内容）；拷贝和摘要在解冻之后从定格做（BUG-045）。
pub(crate) mod audit;
mod capture;
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
    verified: Option<(BrokerCheckpointCoverage, BrokerCapture)>,
}
/// 冻结那一刻的两棵树：目录结构和每个文件的句柄，外加当时的水位和两个源根的身份。
/// 不引用写者实例：写者解冻、照常改文件之后，从这里导出的仍是冻结那一刻（BUG-045）。
pub struct BrokerCapture {
    vault: capture::Captured,
    data: capture::Captured,
    watermark: OwnerWatermark,
    sources: roots::Sources,
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
    ///
    /// 先定格两棵树（逐层列目录、逐个打开文件留句柄、记身份，不读内容），再核重放和回执，核完逐个 stat
    /// 确认两棵树和定格时一样：核过的就是定格的那一刻。重放和回执都有固定上限（`writer::disk` 的
    /// `REPLAY_COUNT`），这一步在窗口里不随历史变长（BUG-045）；拷贝在窗口之外从定格做。
    pub fn verify_audit(
        &mut self,
        mut verify: impl FnMut(BrokerCheckpointAudit<'_>, u64, &str) -> Result<(), BrokerError>,
    ) -> Result<BrokerCheckpointCoverage, BrokerError> {
        self.verified = None;
        let watermark = self.watermark()?;
        let capture = self.capture(watermark)?;
        replays::verify(&self.broker, self.deadline)?;
        let coverage = audit::verify(&self.broker, self.deadline, &mut verify)?;
        self.broker
            .writer
            .check(&self.broker.root, &self.broker.data)?;
        capture
            .vault
            .matches(&self.broker.root, VAULT_QUARANTINE, self.deadline)?;
        capture
            .data
            .matches(&self.broker.data, DATA_QUARANTINE, self.deadline)?;
        if capture.watermark != self.watermark()? {
            return Err(BrokerError::CaptureChanged);
        }
        self.verified = Some((coverage, capture));
        Ok(coverage)
    }
    /// 交出核对过的定格；之后可以解冻写者，在窗口之外用 [`BrokerCapture::export`] 导出。只能交出一次。
    pub fn take_capture(&mut self) -> Result<BrokerCapture, BrokerError> {
        check_deadline(self.deadline)?;
        self.verified
            .take()
            .map(|(_, capture)| capture)
            .ok_or(BrokerError::IncompleteCapture)
    }
    /// 不先解冻、直接在窗口里从定格导出，只给本 crate 的用例用：生产路径这样调就把整树拷贝放回了冻结窗口
    /// （BUG-045）。协调器走 `take_capture` → 解冻 → [`BrokerCapture::export`]。
    #[cfg(test)]
    pub(crate) fn export(
        &self,
        vault: &Path,
        data: &Path,
    ) -> Result<SealedOwnerCheckpoint, BrokerError> {
        let (_, capture) = self
            .verified
            .as_ref()
            .ok_or(BrokerError::IncompleteCapture)?;
        capture.export(vault, data, self.deadline)
    }
    fn capture(&self, watermark: OwnerWatermark) -> Result<BrokerCapture, BrokerError> {
        self.broker
            .writer
            .check(&self.broker.root, &self.broker.data)?;
        let mut budget = capture::Budget::from_process();
        let vault = capture::capture(
            &self.broker.root,
            VAULT_QUARANTINE,
            &mut budget,
            self.deadline,
        )?;
        let data = capture::capture(
            &self.broker.data,
            DATA_QUARANTINE,
            &mut budget,
            self.deadline,
        )?;
        capture::check_pair(&vault, &data)?;
        self.broker
            .writer
            .check(&self.broker.root, &self.broker.data)?;
        let (source_vault, source_data) = self.broker.writer.checkpoint_paths();
        let sources = roots::Sources::new(
            source_vault,
            &self.broker.root,
            source_data,
            &self.broker.data,
        )?;
        check_deadline(self.deadline)?;
        Ok(BrokerCapture {
            vault,
            data,
            watermark,
            sources,
        })
    }
    /// 无论冻结/核对/导出在哪一步失败，都返还原 owner；不重开、不恢复或清理源。
    #[must_use]
    pub fn thaw(self) -> FsVaultBroker {
        self.broker
    }
}
impl BrokerCapture {
    /// 导出到两个已批准、私有、空且不与源重叠的目录；没有覆写或隐式清理。
    /// 全量物化保留逻辑/物理绑定原字节；恢复只能再走 owner 的新根重绑定协议。
    /// 本票据不替代组合根逐对象 inclusion plan 与最终传输源封存。
    pub fn export(
        &self,
        vault: &Path,
        data: &Path,
        deadline: Instant,
    ) -> Result<SealedOwnerCheckpoint, BrokerError> {
        let target = roots::Targets::new(&self.sources, vault, data)?;
        target.check()?;
        self.vault.unchanged(deadline)?;
        self.data.unchanged(deadline)?;
        let actual_vault = self.vault.copy(&target.vault, deadline)?;
        let actual_data = self.data.copy(&target.data, deadline)?;
        if tree::scan(&target.vault, VAULT_QUARANTINE, deadline)? != actual_vault
            || tree::scan(&target.data, DATA_QUARANTINE, deadline)? != actual_data
        {
            return Err(BrokerError::CaptureChanged);
        }
        target.check()?;
        FsVaultBroker::sync_directory(&target.vault, Path::new("."))?;
        FsVaultBroker::sync_directory(&target.data, Path::new("."))?;
        check_deadline(deadline)?;
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
            watermark: self.watermark.clone(),
        })
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
