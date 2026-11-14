//! 未完成标记先于正文创建；只有确认回收或成功交接才移除，进程崩溃也不会放行下一次。
use super::{
    Result, StagingError,
    io::{Directory, Lease},
};

pub(super) struct Lifecycle {
    lease: Lease,
    marker: Directory,
    name: String,
    removing_marker: bool,
    pub published: bool,
    pub finished: bool,
}
impl Lifecycle {
    pub fn begin(parent: &Directory, key: &str) -> Result<Self> {
        let lease = parent.acquire_lease()?;
        // 旧版本留下的临时目录同样不能静默越过；此处不猜测它的身份后删除。
        for name in parent.names(super::MAX_NODES)? {
            if name.starts_with("incomplete-")
                || name.starts_with("export-")
                || name.starts_with("pending-")
            {
                return Err(StagingError::CleanupRequired);
            }
        }
        let name = format!("incomplete-{key}");
        let marker = parent.create_dir(&name)?;
        lease.check(parent)?;
        Ok(Self {
            lease,
            marker,
            name,
            removing_marker: false,
            published: false,
            finished: false,
        })
    }
    pub fn check(&self, parent: &Directory) -> Result<()> {
        self.lease.check(parent)?;
        if self.removing_marker && !parent.exists(&self.name)? {
            return Ok(());
        }
        self.marker.check_child(parent, &self.name)?;
        if !self.marker.names(1)?.is_empty() {
            return Err(StagingError::Changed);
        }
        Ok(())
    }
    pub fn finish(&mut self, parent: &Directory) -> Result<()> {
        self.check(parent)?;
        self.removing_marker = true;
        self.marker.discard(parent, &self.name, None)?;
        self.lease.check(parent)?;
        self.finished = true;
        Ok(())
    }
}
#[cfg(test)]
#[path = "../../../tests/private/staging_lifecycle.rs"]
mod tests;
