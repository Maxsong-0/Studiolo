//! 从根逐段打开目录；不跟随任何祖先链接，也不复用其他实例的锁 FD。

use crate::BrokerError;
use cap_std::fs::{Dir, MetadataExt as _};
use rustix::fs::{FlockOperation, Mode, OFlags, flock, open, openat};
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    path::{Component, Path, PathBuf},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Identity {
    pub(crate) dev: u64,
    pub(crate) ino: u64,
}

impl Identity {
    pub(crate) fn directory(dir: &Dir) -> Result<Self, BrokerError> {
        let m = dir
            .dir_metadata()
            .map_err(|e| BrokerError::io("stat writer directory", e))?;
        if !m.is_dir() {
            return Err(BrokerError::WriterBindingMismatch);
        }
        Ok(Self {
            dev: m.dev(),
            ino: m.ino(),
        })
    }
    pub(crate) fn file(m: &cap_std::fs::Metadata) -> Self {
        Self {
            dev: m.dev(),
            ino: m.ino(),
        }
    }
}

pub(crate) struct DirectoryPin {
    path: PathBuf,
    identity: Identity,
}

impl DirectoryPin {
    pub(crate) fn new(path: PathBuf, dir: &Dir) -> Result<Self, BrokerError> {
        Ok(Self {
            path,
            identity: Identity::directory(dir)?,
        })
    }
    pub(crate) fn check(&self) -> Result<(), BrokerError> {
        if Identity::directory(&walk(&self.path, false)?)? != self.identity {
            return Err(BrokerError::WriterBindingMismatch);
        }
        Ok(())
    }
}

pub(crate) fn absolute(path: &Path) -> Result<PathBuf, BrokerError> {
    if !path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(BrokerError::WriterBindingMismatch);
    }
    Ok(path.components().collect())
}

pub(crate) fn walk(path: &Path, create: bool) -> Result<Dir, BrokerError> {
    let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let fd = open("/", flags, Mode::empty())
        .map_err(|e| BrokerError::io("open writer anchor", e.into()))?;
    let mut dir = Dir::from_std_file(File::from(fd));
    for component in path.components() {
        let Component::Normal(name) = component else {
            continue;
        };
        match openat(&dir, name, flags, Mode::empty()) {
            Ok(fd) => dir = Dir::from_std_file(File::from(fd)),
            Err(e) if create && e == rustix::io::Errno::NOENT => {
                match dir.create_dir(name) {
                    Ok(()) => crate::FsVaultBroker::sync_directory(&dir, Path::new("."))?,
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(e) => return Err(BrokerError::io("create writer directory", e)),
                }
                let fd = openat(&dir, name, flags, Mode::empty())
                    .map_err(|e| BrokerError::io("open created writer directory", e.into()))?;
                dir = Dir::from_std_file(File::from(fd));
            }
            Err(e) => return Err(BrokerError::io("open writer directory", e.into())),
        }
    }
    Ok(dir)
}

pub(crate) fn acquire(root: &Dir) -> Result<File, BrokerError> {
    // dup 仅用于把同一次独立 open 的所有权交给租约；不会获取第二把锁。
    let file = root
        .try_clone()
        .map_err(|e| BrokerError::io("retain writer lease", e))?
        .into_std_file();
    flock(&file, FlockOperation::NonBlockingLockExclusive).map_err(|e| {
        if e == rustix::io::Errno::WOULDBLOCK {
            BrokerError::WriterBusy
        } else {
            BrokerError::io("acquire canonical writer lease", e.into())
        }
    })?;
    Ok(file)
}

pub(crate) fn has_ancestor(path: &Path, identity: Identity) -> Result<bool, BrokerError> {
    let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let fd = open("/", flags, Mode::empty())
        .map_err(|e| BrokerError::io("open writer ancestry", e.into()))?;
    let mut dir = Dir::from_std_file(File::from(fd));
    if Identity::directory(&dir)? == identity {
        return Ok(true);
    }
    for component in path.components() {
        let Component::Normal(name) = component else {
            continue;
        };
        match openat(&dir, name, flags, Mode::empty()) {
            Ok(fd) => dir = Dir::from_std_file(File::from(fd)),
            Err(e) if e == rustix::io::Errno::NOENT => return Ok(false),
            Err(e) => return Err(BrokerError::io("inspect writer ancestry", e.into())),
        }
        if Identity::directory(&dir)? == identity {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(crate) fn child(root: &Dir, path: &str) -> Result<Dir, BrokerError> {
    let mut dir = root
        .try_clone()
        .map_err(|e| BrokerError::io("retain writer directory", e))?;
    for component in Path::new(path).components() {
        match component {
            Component::CurDir => {}
            Component::Normal(name) => {
                let fd = openat(
                    &dir,
                    name,
                    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                    Mode::empty(),
                )
                .map_err(|e| BrokerError::io("open writer layout", e.into()))?;
                dir = Dir::from_std_file(File::from(fd));
            }
            _ => return Err(BrokerError::CorruptWriterState),
        }
    }
    Ok(dir)
}
