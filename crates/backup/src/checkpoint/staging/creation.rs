//! 部分构造也持有未完成标记；先尽力回收所有原根，全部确认后才解除门禁。
use super::{Result, StagingArea, StagingError, io::Directory, lifecycle::Lifecycle};

pub(super) fn layout(
    parent: &Directory,
    key: &str,
    export: &str,
    pending: &str,
) -> Result<(Lifecycle, Directory, Directory, Directory)> {
    let mut lifecycle = Lifecycle::begin(parent, key)?;
    let mut exports = None;
    let mut view = None;
    let result = (|| {
        exports = Some(parent.create_dir(export)?);
        for name in super::OWNER_DIRECTORIES {
            exports
                .as_ref()
                .ok_or(StagingError::Unavailable)?
                .create_dir(name)?;
        }
        view = Some(parent.create_dir(pending)?);
        let data = view
            .as_ref()
            .ok_or(StagingError::Unavailable)?
            .create_dir("data")?;
        lifecycle.check(parent)?;
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
            let mut clean = true;
            for (root, name) in [(exports.as_ref(), export), (view.as_ref(), pending)] {
                // mkdir/fsync 后、拿到句柄之前也可能出错；无原句柄则保留标记。
                let removed = match root {
                    Some(root) => root.discard(parent, name, None),
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
            if clean && lifecycle.finish(parent).is_ok() {
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
        if self.lifecycle.finished {
            return Ok(());
        }
        // 换过的父路径或租约不能成为删除别人的目录的理由。
        self.parent
            .check_path(&self.parent_path)
            .map_err(|_| StagingError::CleanupFailed)?;
        self.lifecycle
            .check(&self.parent)
            .map_err(|_| StagingError::CleanupFailed)?;
        let export = self
            .exports
            .discard(&self.parent, &self.export_name, self.reader);
        let pending = self
            .pending
            .discard(&self.parent, &self.pending_name, self.reader);
        // rename 可能已经生效、但父目录 fsync 返回失败；两个名称都只能删除原 inode。
        let published = if self.lifecycle.published {
            self.pending
                .discard(&self.parent, &self.final_name, self.reader)
        } else {
            Ok(())
        };
        if export.is_err() || pending.is_err() || published.is_err() {
            return Err(StagingError::CleanupFailed);
        }
        self.lifecycle
            .finish(&self.parent)
            .map_err(|_| StagingError::CleanupFailed)
    }
}
