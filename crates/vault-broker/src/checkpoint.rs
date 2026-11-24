//! 实际唯一 Broker 整体进入封存窗口；原租约与全部可变接口一并保留，解冻返还原实例。
//! 窗口里只核水位、定格两棵树（打开文件留句柄，不读内容）、复核；拷贝和摘要在解冻之后从定格做（BUG-045），
//! 逐条核回执在解冻之后、导出的副本上做（BUG-050）。
pub(crate) mod audit;
mod capture;
mod copy;
mod exported;
mod files;
mod open_files;
mod replays;
mod roots;
mod tree;

use crate::{
    BrokerError, FsVaultBroker,
    residue::{DATA_QUARANTINE, VAULT_QUARANTINE},
    writer::Binding,
};
pub use audit::{BrokerCheckpointAudit, BrokerCheckpointCoverage};
pub use exported::ExportedBroker;
pub use open_files::raise_open_file_limit;
use std::{path::Path, time::Instant};
use studiolo_core::checkpoint::OwnerWatermark;

/// 冻结的实际 owner；没有第二写者、可克隆句柄或绕过冻结的普通读写接口。
pub struct FrozenBroker {
    broker: FsVaultBroker,
    deadline: Instant,
    captured: Option<BrokerCapture>,
}
/// 冻结那一刻的两棵树：目录结构、目录身份和每个文件的句柄，外加当时的水位、写者绑定和两个源根的身份。
/// 不引用写者实例：写者解冻、照常改文件之后，从这里导出的仍是冻结那一刻（BUG-045）。
pub struct BrokerCapture {
    vault: capture::Captured,
    data: capture::Captured,
    watermark: OwnerWatermark,
    binding: Binding,
    sources: roots::Sources,
}
impl FsVaultBroker {
    /// 交出完整实例并保留原租约。构造不丢弃 owner；检查失败仍能经 thaw 返还。
    pub fn freeze_checkpoint(self, deadline: Instant) -> FrozenBroker {
        FrozenBroker {
            broker: self,
            deadline,
            captured: None,
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
    /// 窗口里定格两棵树（逐层列目录、逐个打开文件留句柄、记身份，不读内容），再逐个 stat 复核和定格时一样，
    /// 前后水位也一样：定格的就是这一刻。失败会撤销先前的定格。
    ///
    /// 窗口里不读回执（BUG-050）：逐条核回执随编辑数涨，放在解冻之后、从定格导出的副本上做
    /// （[`ExportedBroker::verify`]）。这里不收核对回调，窗口里想核也核不了。
    pub fn capture(&mut self) -> Result<(), BrokerError> {
        self.captured = None;
        let watermark = self.watermark()?;
        let capture = self.capture_trees(watermark)?;
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
        self.captured = Some(capture);
        Ok(())
    }
    /// 交出定格；之后可以解冻写者，在窗口之外用 [`BrokerCapture::export`] 导出、核对。只能交出一次。
    pub fn take_capture(&mut self) -> Result<BrokerCapture, BrokerError> {
        check_deadline(self.deadline)?;
        self.captured.take().ok_or(BrokerError::IncompleteCapture)
    }
    fn capture_trees(&self, watermark: OwnerWatermark) -> Result<BrokerCapture, BrokerError> {
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
            binding: self.broker.writer.binding.clone(),
            sources,
        })
    }
    /// 无论冻结、定格在哪一步失败，都返还原 owner；不重开、不恢复或清理源。
    #[must_use]
    pub fn thaw(self) -> FsVaultBroker {
        self.broker
    }
}
impl BrokerCapture {
    /// 导出到两个已批准、私有、空且不与源重叠的目录；没有覆写或隐式清理。
    /// 全量物化保留逻辑/物理绑定原字节；恢复只能再走 owner 的新根重绑定协议。
    /// 交回的副本还没核过回执、没有票据；票据只由 [`ExportedBroker::verify`] 交出，
    /// 它也不替代组合根逐对象 inclusion plan 与最终传输源封存。
    ///
    /// ```
    /// # use studiolo_vault_broker::{BrokerCapture, BrokerError};
    /// # use studiolo_core::checkpoint::SealedOwnerCheckpoint;
    /// # use std::{path::Path, time::Instant};
    /// fn seal(
    ///     capture: &BrokerCapture,
    ///     vault: &Path,
    ///     data: &Path,
    ///     deadline: Instant,
    /// ) -> Result<SealedOwnerCheckpoint, BrokerError> {
    ///     let exported = capture.export(vault, data, deadline)?;
    ///     Ok(exported.verify(deadline, |_, _, _| Ok(()))?.0)
    /// }
    /// ```
    ///
    /// 没核过就拿不到票据：
    ///
    /// ```compile_fail
    /// # use studiolo_vault_broker::{BrokerCapture, BrokerError};
    /// # use studiolo_core::checkpoint::SealedOwnerCheckpoint;
    /// # use std::{path::Path, time::Instant};
    /// fn seal(
    ///     capture: &BrokerCapture,
    ///     vault: &Path,
    ///     data: &Path,
    ///     deadline: Instant,
    /// ) -> Result<SealedOwnerCheckpoint, BrokerError> {
    ///     capture.export(vault, data, deadline)
    /// }
    /// ```
    pub fn export(
        &self,
        vault: &Path,
        data: &Path,
        deadline: Instant,
    ) -> Result<ExportedBroker, BrokerError> {
        let target = roots::Targets::new(&self.sources, vault, data)?;
        target.check()?;
        self.vault.unchanged(deadline)?;
        self.data.unchanged(deadline)?;
        let mut originals = exported::Originals::default();
        let actual_vault = self.vault.copy(&target.vault, &mut originals, deadline)?;
        let actual_data = self.data.copy(&target.data, &mut originals, deadline)?;
        if tree::scan(&target.vault, VAULT_QUARANTINE, deadline)? != actual_vault
            || tree::scan(&target.data, DATA_QUARANTINE, deadline)? != actual_data
        {
            return Err(BrokerError::CaptureChanged);
        }
        target.check()?;
        FsVaultBroker::sync_directory(&target.vault, Path::new("."))?;
        FsVaultBroker::sync_directory(&target.data, Path::new("."))?;
        check_deadline(deadline)?;
        Ok(ExportedBroker::new(
            target,
            originals,
            self.binding.clone(),
            self.watermark.clone(),
        ))
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
