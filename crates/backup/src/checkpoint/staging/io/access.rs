//! 路径打开时固定的权限角色；读取角色没有创建、删除或改变权限的能力。
use super::super::ReaderIdentity;
use super::{Result, StagingError};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Access {
    Source,
    Private,
    Publisher(ReaderIdentity),
    SharedParent(ReaderIdentity),
    SharingRoot(ReaderIdentity),
    Shared(ReaderIdentity),
}
impl Access {
    pub fn directory(self, uid: u32, gid: u32, mode: u32) -> bool {
        let mode = mode & 0o7777;
        match self {
            Self::Source => uid == rustix::process::geteuid().as_raw(),
            Self::Private => {
                uid == rustix::process::geteuid().as_raw() && matches!(mode, 0o700 | 0o500)
            }
            Self::Publisher(id) | Self::SharedParent(id) => {
                Self::identity(id, uid, gid) && mode == 0o750
            }
            Self::SharingRoot(id) => Self::identity(id, uid, gid) && mode == 0o500,
            Self::Shared(id) => Self::identity(id, uid, gid) && mode == 0o550,
        }
    }
    pub fn file(self, uid: u32, gid: u32, mode: u32) -> bool {
        match self {
            Self::Private => {
                uid == rustix::process::geteuid().as_raw() && matches!(mode & 0o7777, 0o600 | 0o400)
            }
            Self::Shared(id) | Self::SharingRoot(id) => {
                Self::identity(id, uid, gid) && mode & 0o7777 == 0o440
            }
            _ => false,
        }
    }
    pub fn child(self) -> Self {
        match self {
            Self::SharedParent(id) | Self::SharingRoot(id) | Self::Shared(id) => Self::Shared(id),
            _ => Self::Private,
        }
    }
    pub fn readonly_directory(self) -> u32 {
        if matches!(self, Self::Shared(_)) {
            0o550
        } else {
            0o500
        }
    }
    pub fn readonly_file(self) -> u32 {
        if matches!(self, Self::Shared(_) | Self::SharingRoot(_)) {
            0o440
        } else {
            0o400
        }
    }
    pub fn require_writer(self) -> Result<()> {
        match self {
            Self::Private => Ok(()),
            Self::Publisher(id) if id.producer_uid == rustix::process::geteuid().as_raw() => Ok(()),
            _ => Err(StagingError::UnsafeRoot),
        }
    }
    fn identity(id: ReaderIdentity, uid: u32, gid: u32) -> bool {
        uid == id.producer_uid && gid == id.reader_gid
    }
}
