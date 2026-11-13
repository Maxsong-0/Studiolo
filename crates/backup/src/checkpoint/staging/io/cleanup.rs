//! 只回收本次新建目录的原 FD；根被换掉则拒绝，绝不沿链接进入其他目录。
use super::{Directory, Mode, component, error, fs};
use crate::checkpoint::staging::{MAX_NODES, Result, StagingError};

impl Directory {
    pub fn discard(&self, parent: &Self, name: &str) -> Result<()> {
        self.access.require_writer()?;
        parent.access.require_writer()?;
        component(name)?;
        if !parent.exists(name)? {
            return Ok(());
        }
        self.check_child(parent, name)?;
        let mut remaining = MAX_NODES;
        self.empty(&mut remaining, 0)?;
        self.check_child(parent, name)?;
        fs::unlinkat(&parent.fd, name, fs::AtFlags::REMOVEDIR).map_err(error)?;
        parent.sync()
    }
    fn empty(&self, remaining: &mut u64, depth: usize) -> Result<()> {
        if depth > 64 {
            return Err(StagingError::Limit);
        }
        fs::fchmod(&self.fd, Mode::RWXU).map_err(error)?;
        let names = self.names(*remaining)?;
        *remaining -= names.len() as u64;
        for name in &names {
            let stat = fs::statat(&self.fd, name, fs::AtFlags::SYMLINK_NOFOLLOW).map_err(error)?;
            if super::FileType::from_raw_mode(stat.st_mode) == super::FileType::Directory {
                let child = self.child(name)?;
                child.empty(remaining, depth + 1)?;
                child.check_child(self, name)?;
                fs::unlinkat(&self.fd, name, fs::AtFlags::REMOVEDIR).map_err(error)?;
            } else {
                // 普通文件即使多了一条链接也只删除本次副本的目录项，不碰链接另一端。
                fs::unlinkat(&self.fd, name, fs::AtFlags::empty()).map_err(error)?;
            }
        }
        self.sync()
    }
}
