//! 入口文件系统 Adapter；保留父目录 FD，清理不跟随当前路径指向的新父目录。

use super::IngressError;
use rustix::{
    fs::{AtFlags, FileType, Mode, OFlags, Stat, chmodat, open, statat, unlinkat},
    io::Errno,
    process::{getegid, geteuid, getgid, getuid},
};
use std::{
    ffi::OsStr,
    fs::{self, File},
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};

type Identity = (u64, u64);

struct Directory {
    path: PathBuf,
    identities: Vec<Identity>,
}

impl Directory {
    fn read(path: &Path) -> Result<Self, IngressError> {
        normal(path)?;
        let mut current = PathBuf::new();
        let mut identities = Vec::new();
        for part in path.components() {
            current.push(part);
            let meta = fs::symlink_metadata(&current).map_err(|_| IngressError::Configuration)?;
            if !meta.is_dir() || meta.file_type().is_symlink() {
                return Err(IngressError::Configuration);
            }
            identities.push((meta.dev(), meta.ino()));
        }
        Ok(Self {
            path: path.to_path_buf(),
            identities,
        })
    }

    fn identity(&self) -> Identity {
        *self.identities.last().expect("绝对目录含根对象")
    }

    fn revalidate(&self) -> Result<(), IngressError> {
        if Self::read(&self.path)?.identities != self.identities {
            return Err(IngressError::Configuration);
        }
        Ok(())
    }
}

pub(super) struct Layout {
    path: PathBuf,
    parent: Directory,
    descriptor: File,
    roots: Vec<Directory>,
}

impl Layout {
    pub(super) fn prepare(path: &Path, roots: &[&Path]) -> Result<Self, IngressError> {
        normal(path)?;
        if path.as_os_str().as_encoded_bytes().len() > 103 || path.file_name().is_none() {
            return Err(IngressError::Configuration);
        }
        let parent = Directory::read(path.parent().ok_or(IngressError::Configuration)?)?;
        let descriptor = File::from(
            open(
                &parent.path,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|_| IngressError::Configuration)?,
        );
        let roots = roots
            .iter()
            .map(|path| Directory::read(path))
            .collect::<Result<Vec<_>, _>>()?;
        let layout = Self {
            path: path.to_path_buf(),
            parent,
            descriptor,
            roots,
        };
        layout.revalidate()?;
        Ok(layout)
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    fn name(&self) -> &OsStr {
        self.path.file_name().expect("准备阶段已检查文件名")
    }

    fn directories(&self) -> Result<(), IngressError> {
        if getuid() != geteuid() || getgid() != getegid() {
            return Err(IngressError::Configuration);
        }
        self.parent.revalidate()?;
        let meta = self
            .descriptor
            .metadata()
            .map_err(|_| IngressError::Configuration)?;
        if !meta.is_dir()
            || (meta.dev(), meta.ino()) != self.parent.identity()
            || meta.mode() & 0o7777 != 0o700
            || meta.uid() != getuid().as_raw()
        {
            return Err(IngressError::Configuration);
        }
        for root in &self.roots {
            root.revalidate()?;
            // 以对象身份识别大小写别名和相同或嵌套根，不只比较字符串。
            if root.identities.contains(&self.parent.identity())
                || self.parent.identities.contains(&root.identity())
            {
                return Err(IngressError::Configuration);
            }
        }
        Ok(())
    }

    pub(super) fn revalidate(&self) -> Result<(), IngressError> {
        self.directories()?;
        match statat(&self.descriptor, self.name(), AtFlags::SYMLINK_NOFOLLOW) {
            Err(Errno::NOENT) => Ok(()),
            _ => Err(IngressError::Configuration),
        }
    }

    pub(super) fn claim(self) -> Result<Cleanup, IngressError> {
        let stat = self.socket()?;
        let cleanup = Cleanup {
            identity: identity(&stat),
            layout: self,
        };
        cleanup.layout.directories()?;
        cleanup.current()?;
        // 锁定 rustix 在 Linux 不支持 nofollow chmod；此处依赖同 UID 的 0700 父目录。
        // chmod 与清理都相对已保留 FD，并在前后核对 inode；不宣称对抗同 UID 竞争。
        chmodat(
            &cleanup.layout.descriptor,
            cleanup.layout.name(),
            Mode::RUSR | Mode::WUSR,
            AtFlags::empty(),
        )
        .map_err(|_| IngressError::Configuration)?;
        cleanup.revalidate()?;
        Ok(cleanup)
    }

    fn socket(&self) -> Result<Stat, IngressError> {
        let stat = statat(&self.descriptor, self.name(), AtFlags::SYMLINK_NOFOLLOW)
            .map_err(|_| IngressError::Configuration)?;
        if FileType::from_raw_mode(stat.st_mode) != FileType::Socket
            || stat.st_uid != getuid().as_raw()
            || stat.st_nlink != 1
        {
            return Err(IngressError::Configuration);
        }
        Ok(stat)
    }
}

pub(super) struct Cleanup {
    layout: Layout,
    identity: Identity,
}

impl Cleanup {
    fn current(&self) -> Result<Stat, IngressError> {
        let stat = self.layout.socket()?;
        if identity(&stat) != self.identity {
            return Err(IngressError::Configuration);
        }
        Ok(stat)
    }

    pub(super) fn revalidate(&self) -> Result<(), IngressError> {
        self.layout.directories()?;
        if self.current()?.st_mode & 0o7777 != 0o600 {
            return Err(IngressError::Configuration);
        }
        Ok(())
    }
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        if self.current().is_ok() {
            let _ = unlinkat(
                &self.layout.descriptor,
                self.layout.name(),
                AtFlags::empty(),
            );
        }
    }
}

fn identity(stat: &Stat) -> Identity {
    #[cfg(target_os = "macos")]
    let device = stat.st_dev as u64;
    #[cfg(not(target_os = "macos"))]
    let device = stat.st_dev;
    (device, stat.st_ino)
}

fn normal(path: &Path) -> Result<(), IngressError> {
    if !path.to_str().is_some_and(|value| {
        value.starts_with('/')
            && !value.chars().any(char::is_control)
            && (value == "/"
                || value[1..]
                    .split('/')
                    .all(|part| !part.is_empty() && part != "." && part != ".."))
    }) {
        return Err(IngressError::Configuration);
    }
    Ok(())
}
