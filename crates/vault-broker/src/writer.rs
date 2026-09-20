//! 单写者租约、不可变配对与有界持久记录的私有边界。

mod binding;
pub(crate) mod disk;
mod lease;
mod opening;

use crate::{BrokerError, FsVaultBroker, VaultUuid};
use cap_std::fs::Dir;
use std::{
    path::Path,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};

pub(crate) use binding::Binding;
pub(crate) use lease::child as directory;
pub(crate) use lease::{DirectoryPin, Identity};

/// 持久写者状态的观测值，不代表授权或当前文件仍为某次提交的内容。
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub struct WriterStatus {
    /// 已完成并持久发布的最大提交序号；旧格式回执不增加此水位。
    pub committed_through: u64,
    /// 存在尚未解决的操作，不能启动另一笔 canonical 安装。
    pub unresolved: bool,
    /// 精确阶段；不授予任何协调或写入权限。
    pub phase: crate::WriterPhase,
    /// 是否禁止完整捕获和单页读取；Prepared 只阻止新写入。
    pub read_blocked: bool,
}

pub(crate) struct Writer {
    // 租约来自本实例独立 open 的根；销毁时显式解锁，不受 root 副本寿命延长。
    _lease: lease::LeaseGuard,
    root: DirectoryPin,
    data: DirectoryPin,
    control: Identity,
    marker: disk::FilePin,
    root_binding: disk::FilePin,
    data_binding: disk::FilePin,
    directories: Vec<(bool, &'static str, Identity)>,
    pub(crate) binding: Binding,
    uncertain: AtomicBool,
    highest_committed: AtomicU64,
    rollback: AtomicBool,
    #[cfg(test)]
    pub(crate) fault: std::sync::atomic::AtomicU8,
}

impl Writer {
    pub(crate) fn open(
        vault: &Path,
        data: &Path,
        initialize: bool,
    ) -> Result<(Dir, Dir, VaultUuid, Self), BrokerError> {
        opening::open(vault, data, initialize)
    }

    pub(crate) fn check(&self, root: &Dir, data: &Dir) -> Result<(), BrokerError> {
        if self.rollback.load(Ordering::Acquire) {
            return Err(BrokerError::CorruptWriterState);
        }
        self.root.check().map_err(|_| BrokerError::RootChanged)?;
        self.data
            .check()
            .map_err(|_| BrokerError::WriterBindingMismatch)?;
        let control = lease::child(root, ".workbench")?;
        if Identity::directory(&control)? != self.control {
            return Err(BrokerError::WriterBindingMismatch);
        }
        let marker = self
            .marker
            .check(&control, "vault.json", disk::IDENTITY_LIMIT)?;
        let left =
            self.root_binding
                .check(&control, "broker-binding.json", disk::IDENTITY_LIMIT)?;
        let right = self
            .data_binding
            .check(data, "broker-binding.json", disk::IDENTITY_LIMIT)?;
        if crate::VaultIdentity::decode(&marker)? != self.binding.vault_uuid
            || disk::decode::<Binding>(&left)? != self.binding
            || disk::decode::<Binding>(&right)? != self.binding
        {
            return Err(BrokerError::WriterBindingMismatch);
        }
        for (canonical, path, expected) in &self.directories {
            let dir = lease::child(if *canonical { root } else { data }, path)?;
            if Identity::directory(&dir)? != *expected {
                return Err(BrokerError::WriterBindingMismatch);
            }
        }
        Ok(())
    }

    // 只接受完整验证的日志水位或成功持久提交；未决序号不代表已提交。
    pub(crate) fn observe_committed(&self, committed: u64) -> Result<(), BrokerError> {
        let previous = self
            .highest_committed
            .fetch_max(committed, Ordering::AcqRel);
        if committed < previous {
            self.rollback.store(true, Ordering::Release);
        }
        if self.rollback.load(Ordering::Acquire) {
            return Err(BrokerError::CorruptWriterState);
        }
        Ok(())
    }

    pub(crate) fn poison(&self) {
        self.uncertain.store(true, Ordering::Release);
    }

    pub(crate) fn uncertain(&self) -> bool {
        self.uncertain.load(Ordering::Acquire)
    }
}

impl FsVaultBroker {
    /// 仅打开已初始化的配对存储；缺失和部分初始化不会被自动修复。
    pub fn open_existing(vault_root: &Path, data_root: &Path) -> Result<Self, BrokerError> {
        Self::open_writer(vault_root, data_root, false)
    }

    pub(crate) fn open_writer(
        vault: &Path,
        data: &Path,
        initialize: bool,
    ) -> Result<Self, BrokerError> {
        let (root, data, vault_uuid, writer) = Writer::open(vault, data, initialize)?;
        let broker = Self {
            root,
            data,
            vault_uuid,
            writer,
            capture_instance: std::sync::Arc::new(()),
            recovery_review: AtomicU64::new(0),
            history_pin: std::sync::Mutex::new(None),
            maintenance_pin: std::sync::Mutex::new(None),
            file_operation_pin: std::sync::Mutex::new(None),
            read_guard: std::sync::Mutex::new(()),
        };
        crate::maintenance::blocked(&broker)?;
        crate::mutation::reopen(&broker)?;
        crate::maintenance::reopen(&broker)?;
        crate::batch::barrier::clear_if_idle(&broker)?;
        Ok(broker)
    }

    /// 读取日志快照；不安装文件、不清理未决操作，也不授予写入权限。
    pub fn write_status(&self) -> Result<WriterStatus, BrokerError> {
        let state = crate::mutation::load(self)?;
        use crate::{WriterPhase, mutation::record::Phase};
        let phase = if self.writer.uncertain() {
            WriterPhase::Uncertain
        } else if state.legacy_unresolved {
            WriterPhase::LegacyUnknown
        } else if state.batch.is_some() || crate::batch::barrier::blocked(&self.root)? {
            WriterPhase::Installing
        } else if let Some(op) = &state.file_operation {
            crate::file_operation::phase(op.phase)
        } else {
            match state.operation.as_ref().map(|op| op.phase) {
                None => WriterPhase::Ready,
                Some(Phase::Prepared) => WriterPhase::Prepared,
                Some(Phase::Installing) => WriterPhase::Installing,
                Some(Phase::Completed) => WriterPhase::Completed,
            }
        };
        Ok(WriterStatus {
            committed_through: state.committed_through,
            unresolved: state.legacy_unresolved
                || state.operation.is_some()
                || state.file_operation.is_some()
                || state.batch.is_some()
                || crate::batch::barrier::blocked(&self.root)?
                || self.writer.uncertain()
                || crate::maintenance::blocked(self)?,
            phase,
            read_blocked: !matches!(phase, WriterPhase::Ready | WriterPhase::Prepared),
        })
    }
}

// 只读来源独立固定其绝对根，不借用 canonical 的写权限或外部传入 FD。
pub(crate) fn open_blob_source_root(path: &Path) -> Result<(Dir, DirectoryPin), BrokerError> {
    let path = lease::absolute(path)?;
    let root = lease::walk(&path, false)?;
    let pin = DirectoryPin::new(path, &root)?;
    pin.check()?;
    Ok((root, pin))
}
