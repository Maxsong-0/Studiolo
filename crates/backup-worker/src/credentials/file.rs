//! 单链接只读凭据、有界零化读取；文件名只能来自三个固定槽位。
use super::root::{Root, check_deadline, no_acl, readonly_memory};
use crate::{ProcessError, Result};
use rustix::fs::{self, Mode, OFlags};
use std::{
    fs::{File, Metadata},
    io::Read,
    os::unix::fs::MetadataExt,
    time::Instant,
};
use zeroize::Zeroizing;

#[derive(Clone, Copy)]
pub(super) enum Slot {
    Repository,
    Username,
    Password,
}
impl Slot {
    pub fn name(self) -> &'static str {
        match self {
            Self::Repository => "repository-password",
            Self::Username => "rest-username",
            Self::Password => "rest-password",
        }
    }
    fn limit(self) -> usize {
        if matches!(self, Self::Username) {
            256
        } else {
            4096
        }
    }
}
pub(super) struct SecretFile {
    file: File,
    pin: Metadata,
    slot: Slot,
}
impl SecretFile {
    pub fn load(root: &Root, slot: Slot, deadline: Instant) -> Result<(Self, Zeroizing<String>)> {
        let file = open_slot(root, slot)?;
        let (value, pin) = read_checked(&file, root.uid(), slot, deadline)?;
        let checked = Self { file, pin, slot };
        checked.check(root, deadline)?;
        Ok((checked, value))
    }
    pub fn check(&self, root: &Root, deadline: Instant) -> Result<()> {
        check_deadline(deadline)?;
        readonly_memory(&self.file)?;
        no_acl(&self.file)?;
        let after = self
            .file
            .metadata()
            .map_err(|_| ProcessError::Configuration)?;
        let reopened = open_slot(root, self.slot)?
            .metadata()
            .map_err(|_| ProcessError::Configuration)?;
        if !same(&self.pin, &after) || !same(&self.pin, &reopened) {
            return Err(ProcessError::Configuration);
        }
        check_deadline(deadline)
    }
}
fn open_slot(root: &Root, slot: Slot) -> Result<File> {
    let fd = fs::openat(
        root.fd(),
        slot.name(),
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
    )
    .map_err(|_| ProcessError::Configuration)?;
    readonly_memory(&fd)?;
    no_acl(&fd)?;
    Ok(File::from(fd))
}
fn read_checked(
    mut file: &File,
    uid: u32,
    slot: Slot,
    deadline: Instant,
) -> Result<(Zeroizing<String>, Metadata)> {
    check_deadline(deadline)?;
    let pin = file.metadata().map_err(|_| ProcessError::Configuration)?;
    let limit = slot.limit();
    if !pin.is_file()
        || pin.uid() != uid
        || pin.nlink() != 1
        || pin.mode() & 0o7777 != 0o400
        || pin.len() == 0
        || pin.len() > (limit + 1) as u64
    {
        return Err(ProcessError::Configuration);
    }
    let mut bytes = Zeroizing::new(Vec::with_capacity(limit + 1));
    let mut block = Zeroizing::new([0_u8; 512]);
    loop {
        check_deadline(deadline)?;
        let count = file
            .read(&mut *block)
            .map_err(|_| ProcessError::Configuration)?;
        if count == 0 {
            break;
        }
        if count > limit + 1 - bytes.len() {
            return Err(ProcessError::Configuration);
        }
        bytes.extend_from_slice(&block[..count]);
    }
    let after = file.metadata().map_err(|_| ProcessError::Configuration)?;
    if bytes.len() as u64 != pin.len() || !same(&pin, &after) {
        return Err(ProcessError::Configuration);
    }
    let value = parse(&bytes, slot)?;
    check_deadline(deadline)?;
    Ok((value, pin))
}
fn parse(bytes: &[u8], slot: Slot) -> Result<Zeroizing<String>> {
    // 允许录入工具留下一个 LF；其余控制字符/首尾空白拒绝，避免与 restic 解读不一致。
    let bytes = bytes.strip_suffix(b"\n").unwrap_or(bytes);
    let value = std::str::from_utf8(bytes).map_err(|_| ProcessError::Configuration)?;
    if value.is_empty()
        || value.len() > slot.limit()
        || value.trim() != value
        || value.chars().any(char::is_control)
        || (matches!(slot, Slot::Username) && value.contains(':'))
    {
        return Err(ProcessError::Configuration);
    }
    Ok(Zeroizing::new(value.to_owned()))
}
fn same(a: &Metadata, b: &Metadata) -> bool {
    a.dev() == b.dev()
        && a.ino() == b.ino()
        && a.uid() == b.uid()
        && a.gid() == b.gid()
        && a.mode() == b.mode()
        && a.nlink() == b.nlink()
        && a.len() == b.len()
        && a.mtime() == b.mtime()
        && a.mtime_nsec() == b.mtime_nsec()
        && a.ctime() == b.ctime()
        && a.ctime_nsec() == b.ctime_nsec()
}

#[cfg(test)]
#[path = "../../tests/support/credentials_file.rs"]
mod tests;
