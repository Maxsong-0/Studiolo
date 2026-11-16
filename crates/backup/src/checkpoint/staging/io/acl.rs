//! chmod 会改变 POSIX ACL 的 mask，继承的命名主体可能重新获得读取权；因此不接收 ACL。
use crate::checkpoint::staging::{Result, StagingError};
use rustix::fd::AsFd;

#[cfg(target_os = "linux")]
pub(super) fn require_no_acl(fd: &impl AsFd, directory: bool) -> Result<()> {
    for name in ["system.posix_acl_access", "system.posix_acl_default"] {
        if !directory && name == "system.posix_acl_default" {
            continue;
        }
        let mut byte = [0_u8; 1];
        match rustix::fs::fgetxattr(fd, name, &mut byte[..]) {
            Err(rustix::io::Errno::NODATA | rustix::io::Errno::OPNOTSUPP) => {}
            // RANGE 也表示存在 ACL，不能因缓冲小而当成不存在。
            _ => return Err(StagingError::UnsafeRoot),
        }
    }
    Ok(())
}
#[cfg(not(target_os = "linux"))]
pub(super) fn require_no_acl(_: &impl AsFd, _: bool) -> Result<()> {
    Err(StagingError::Unavailable)
}
