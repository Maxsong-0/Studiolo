//! Socket ownership is pinned to a parent FD and inode; cleanup cannot target a replacement.
use rustix::fs::{AtFlags, FileType, Mode, OFlags, Stat, chmodat, open, statat, unlinkat};
use std::{
    fs::File,
    os::unix::net::UnixListener,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

pub(super) struct SocketLease {
    path: PathBuf,
    parent: File,
    identity: (u64, u64),
    released: AtomicBool,
}
impl SocketLease {
    pub fn bind(path: PathBuf) -> Result<(UnixListener, Self), String> {
        let parent = File::from(
            open(
                path.parent().ok_or("Runner socket父目录缺失")?,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|_| "Runner socket父目录不可用")?,
        );
        let name = path.file_name().ok_or("Runner socket名称缺失")?;
        let listener = UnixListener::bind(&path)
            .map_err(|_| "Runner relay socket不可用；旧socket需先核验恢复")?;
        let stat = statat(&parent, name, AtFlags::SYMLINK_NOFOLLOW)
            .map_err(|_| "Runner socket身份不可用")?;
        let lease = Self {
            identity: identity(&stat),
            released: AtomicBool::new(false),
            path,
            parent,
        };
        lease.current()?;
        chmodat(
            &lease.parent,
            lease.name(),
            Mode::RUSR | Mode::WUSR,
            AtFlags::empty(),
        )
        .map_err(|_| "Runner socket权限失败")?;
        lease.current()?;
        listener
            .set_nonblocking(true)
            .map_err(|_| "Runner socket状态失败")?;
        Ok((listener, lease))
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    fn name(&self) -> &std::ffi::OsStr {
        self.path.file_name().expect("validated socket name")
    }
    fn current(&self) -> Result<(), String> {
        let stat = statat(&self.parent, self.name(), AtFlags::SYMLINK_NOFOLLOW)
            .map_err(|_| "Runner socket身份不可用")?;
        if FileType::from_raw_mode(stat.st_mode) != FileType::Socket
            || stat.st_uid != rustix::process::geteuid().as_raw()
            || stat.st_nlink != 1
            || identity(&stat) != self.identity
        {
            return Err("Runner socket身份变化".into());
        }
        Ok(())
    }
    pub fn cleanup(&self) -> Result<(), String> {
        // Once released, never revisit this name: a later socket may reuse the inode number.
        if self.released.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        let result = self.cleanup_owned();
        if result.is_err() {
            self.released.store(false, Ordering::SeqCst);
        }
        result
    }
    fn cleanup_owned(&self) -> Result<(), String> {
        match statat(&self.parent, self.name(), AtFlags::SYMLINK_NOFOLLOW) {
            Err(rustix::io::Errno::NOENT) => return Ok(()),
            Err(_) => return Err("Runner socket清理检查失败".into()),
            Ok(stat) if identity(&stat) != self.identity => return Ok(()),
            Ok(_) => {}
        }
        self.current()?;
        unlinkat(&self.parent, self.name(), AtFlags::empty())
            .map_err(|_| "Runner socket清理失败".into())
    }
}
impl Drop for SocketLease {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}
fn identity(stat: &Stat) -> (u64, u64) {
    #[cfg(target_os = "macos")]
    let device = stat.st_dev as u64;
    #[cfg(not(target_os = "macos"))]
    let device = stat.st_dev;
    (device, stat.st_ino)
}
#[cfg(test)]
#[path = "socket_tests.rs"]
mod tests;
