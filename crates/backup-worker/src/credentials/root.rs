//! systemd 固定凭据根：拒绝可写、可换出和普通目录降级，保留原路径每一级身份。
use super::{
    acl::{self, Kind},
    fault::{CredentialFault, Result},
    mount,
};
use rustix::{
    fd::{AsFd, OwnedFd},
    fs::{self, Mode, OFlags},
};
use std::time::Instant;

pub(super) const ROOT: &str = "/run/credentials/studiolo-backup-worker.service";
const FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);
pub(super) struct Root {
    fd: OwnedFd,
    pins: Vec<(u64, u64, u32, u32, u32)>,
    uid: u32,
}
impl Root {
    pub fn open_credentials(uid: u32, deadline: Instant) -> Result<Self> {
        if uid == 0 || uid != rustix::process::geteuid().as_raw() {
            return Err(CredentialFault::Account);
        }
        let mut fd = fs::open("/", FLAGS, Mode::empty()).map_err(|_| CredentialFault::Parents)?;
        let mut pins = Vec::new();
        for name in [
            None,
            Some("run"),
            Some("credentials"),
            Some("studiolo-backup-worker.service"),
        ] {
            check_deadline(deadline)?;
            // 最后一级是本单元自己的凭据目录，前三级是它的上级；两种失败在日志里分开说。
            let unit = name == Some("studiolo-backup-worker.service");
            let missing = if unit {
                CredentialFault::Missing
            } else {
                CredentialFault::Parents
            };
            if let Some(name) = name {
                fd = fs::openat(&fd, name, FLAGS, Mode::empty()).map_err(|_| missing)?;
            }
            let stat = fs::fstat(&fd).map_err(|_| missing)?;
            if unit {
                // 只有 root 和 worker 读得到（两种形态，逐项核，见 `acl.rs`），而且在不换出的只读内存盘上。
                if !exclusive_access(&fd, Kind::Directory, stat.st_uid, stat.st_mode, uid) {
                    return Err(CredentialFault::DirectoryAccess);
                }
                readonly_memory(&fd)?;
            } else if !no_acl(&fd) || stat.st_uid != 0 || stat.st_mode & 0o7022 != 0 {
                return Err(CredentialFault::Parents);
            }
            pins.push((
                stat.st_dev,
                stat.st_ino,
                stat.st_uid,
                stat.st_gid,
                stat.st_mode,
            ));
        }
        check_deadline(deadline)?;
        Ok(Self { fd, pins, uid })
    }
    pub fn fd(&self) -> &OwnedFd {
        &self.fd
    }
    pub fn uid(&self) -> u32 {
        self.uid
    }
    pub fn check(&self, deadline: Instant) -> Result<()> {
        let reopened = Self::open_credentials(self.uid, deadline)?;
        let original = fs::fstat(&self.fd).map_err(|_| CredentialFault::Changed)?;
        readonly_memory(&self.fd)?;
        if !exclusive_access(
            &self.fd,
            Kind::Directory,
            original.st_uid,
            original.st_mode,
            self.uid,
        ) {
            return Err(CredentialFault::DirectoryAccess);
        }
        if reopened.pins != self.pins
            || self.pins.last()
                != Some(&(
                    original.st_dev,
                    original.st_ino,
                    original.st_uid,
                    original.st_gid,
                    original.st_mode,
                ))
        {
            return Err(CredentialFault::Changed);
        }
        check_deadline(deadline)
    }
}
pub(super) fn check_deadline(deadline: Instant) -> Result<()> {
    if Instant::now() >= deadline {
        Err(CredentialFault::Deadline)
    } else {
        Ok(())
    }
}
/// 上级目录一条 ACL 都不许有：访问 ACL、默认 ACL 都查。
fn no_acl(fd: &impl AsFd) -> bool {
    ["system.posix_acl_access", "system.posix_acl_default"]
        .into_iter()
        .all(|name| absent(fd, name))
}
fn absent(fd: &impl AsFd, name: &str) -> bool {
    let mut byte = [0_u8; 1];
    matches!(
        fs::fgetxattr(fd, name, &mut byte[..]),
        Err(rustix::io::Errno::NODATA | rustix::io::Errno::OPNOTSUPP)
    )
}
/// 目录或文件是不是「只有 root 和 `uid` 读得到」：读出访问 ACL 的原始字节，连同属主和权限位交给
/// `acl::exclusive` 逐项判。默认 ACL（新建的文件会继承它）一律不许有；读 ACL 出别的错、
/// 或者比认得的形态长（缓冲只多留一个字节），都算不是。
pub(super) fn exclusive_access(
    fd: &impl AsFd,
    kind: Kind,
    owner: u32,
    mode: u32,
    uid: u32,
) -> bool {
    let mut raw = [0_u8; acl::EXACT_LEN + 1];
    let read = fs::fgetxattr(fd, "system.posix_acl_access", &mut raw[..]);
    let access = match read {
        Ok(length) if length <= raw.len() => Some(&raw[..length]),
        Err(rustix::io::Errno::NODATA | rustix::io::Errno::OPNOTSUPP) => None,
        _ => return false,
    };
    absent(fd, "system.posix_acl_default")
        && acl::exclusive(kind, owner, mode & 0o7777, access, uid)
}
/// 所在的挂载只读，而且内容不会换出到硬盘：ramfs 从不换出；tmpfs 只有挂载时带 `noswap` 的不换出，
/// 这一项要拿挂载号到挂载表里核（`mount.rs`）。别的文件系统一律不认。
pub(super) fn readonly_memory(fd: &impl AsFd) -> Result<()> {
    let stat = fs::fstatfs(fd).map_err(|_| CredentialFault::Storage)?;
    let mounted = fs::fstatvfs(fd).map_err(|_| CredentialFault::Storage)?;
    if !mounted.f_flag.contains(fs::StatVfsMountFlags::RDONLY) {
        return Err(CredentialFault::Storage);
    }
    let memory = match stat.f_type as u64 {
        0x8584_58f6 => true,
        0x0102_1994 => mount::noswap(fd),
        _ => false,
    };
    if memory {
        Ok(())
    } else {
        Err(CredentialFault::Storage)
    }
}

#[cfg(test)]
#[path = "../../tests/support/credentials_root.rs"]
mod tests;
