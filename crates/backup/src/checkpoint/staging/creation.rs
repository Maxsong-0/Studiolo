//! 部分构造也持有未完成标记；先尽力回收所有原根，全部确认后才解除门禁。
use super::{
    ReaderIdentity, Result, StagingArea, StagingError,
    budget::Budget,
    io::{Directory, JournalFile},
    lifecycle::Lifecycle,
};
use std::time::{Duration, Instant};
use studiolo_core::checkpoint::CheckpointId;

pub(super) fn layout(
    parent: &Directory,
    checkpoint_id: &CheckpointId,
    export: &str,
    pending: &str,
    reader: Option<ReaderIdentity>,
    deadline: Instant,
) -> Result<(Lifecycle, Directory, Directory, Directory)> {
    super::check_deadline(deadline)?;
    let mut lifecycle = Lifecycle::begin(parent, checkpoint_id, reader)?;
    let mut exports = None;
    let mut view = None;
    let result = (|| {
        exports = Some(parent.create_dir(export)?);
        lifecycle.register(
            parent,
            JournalFile::Export,
            exports.as_ref().ok_or(StagingError::Unavailable)?,
        )?;
        for name in super::OWNER_DIRECTORIES {
            super::check_deadline(deadline)?;
            exports
                .as_ref()
                .ok_or(StagingError::Unavailable)?
                .create_dir(name)?;
        }
        view = Some(parent.create_dir(pending)?);
        lifecycle.register(
            parent,
            JournalFile::Pending,
            view.as_ref().ok_or(StagingError::Unavailable)?,
        )?;
        let data = view
            .as_ref()
            .ok_or(StagingError::Unavailable)?
            .create_dir("data")?;
        lifecycle.check(parent)?;
        super::check_deadline(deadline)?;
        Ok(data)
    })();
    match result {
        Ok(data) => Ok((
            lifecycle,
            exports.ok_or(StagingError::Unavailable)?,
            view.ok_or(StagingError::Unavailable)?,
            data,
        )),
        Err(error) => {
            lifecycle
                .check(parent)
                .map_err(|_| StagingError::CleanupFailed)?;
            let mut budget = Budget::until(deadline);
            let mut clean = true;
            for (root, name) in [(exports.as_ref(), export), (view.as_ref(), pending)] {
                // mkdir/fsync 后、拿到句柄之前也可能出错；无原句柄则保留标记。
                let removed = match root {
                    Some(root) => root.discard_within(parent, name, None, &mut budget),
                    None => parent.exists(name).and_then(|exists| {
                        if exists {
                            Err(StagingError::CleanupFailed)
                        } else {
                            Ok(())
                        }
                    }),
                };
                clean &= removed.is_ok();
            }
            if clean && lifecycle.finish(parent, &mut budget).is_ok() {
                Err(error)
            } else {
                Err(StagingError::CleanupFailed)
            }
        }
    }
}
impl StagingArea {
    /// 调用方取消时仍检查真实回收结果；失败后固定标记阻止下一个作业。
    pub fn abort(mut self) -> Result<()> {
        self.cleanup()
    }
    pub(super) fn cleanup(&mut self) -> Result<()> {
        let deadline = self.deadline.min(Instant::now() + Duration::from_secs(30));
        self.cleanup_within(&mut Budget::until(deadline))
    }
    /// 同 `cleanup`；删除的每一步记在给定的预算里（私有测试用它停在任意一步）。
    pub(super) fn cleanup_within(&mut self, budget: &mut Budget) -> Result<()> {
        if self.lifecycle.finished {
            return Ok(());
        }
        budget.check().map_err(|_| StagingError::CleanupFailed)?;
        // 换过的父路径或租约不能成为删除别人的目录的理由。
        self.parent
            .check_path(&self.parent_path)
            .map_err(|_| StagingError::CleanupFailed)?;
        // 已改名成 `retained-` 的先改回原名再删原根：删到一半停下也只留下普通的未完成登记。
        self.lifecycle
            .unretain(&self.parent, budget)
            .map_err(|_| StagingError::CleanupFailed)?;
        let export =
            self.exports
                .discard_within(&self.parent, &self.export_name, self.reader, budget);
        let pending =
            self.pending
                .discard_within(&self.parent, &self.pending_name, self.reader, budget);
        // rename 可能已经生效、但父目录 fsync 返回失败；两个名称都只能删除原 inode。
        let published = if self.lifecycle.published {
            self.pending
                .discard_within(&self.parent, &self.final_name, self.reader, budget)
        } else {
            Ok(())
        };
        if export.is_err() || pending.is_err() || published.is_err() {
            return Err(StagingError::CleanupFailed);
        }
        self.lifecycle
            .finish(&self.parent, budget)
            .map_err(|_| StagingError::CleanupFailed)
    }
}
