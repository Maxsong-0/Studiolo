//! 目录和文件固定身份；本模块是唯一文件创建点，只创建已批准库名的全新目标。
use crate::{DatabaseFile, Error};
use std::{
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
};

pub(crate) struct Binding {
    pub root: PathBuf,
    pub path: PathBuf,
    pub file: DatabaseFile,
    root_pin: File,
    file_pin: File,
}

#[cfg(unix)]
impl Binding {
    pub fn source(root: &Path, file: DatabaseFile) -> Result<Self, Error> {
        Self::source_inner(root, file, false)
    }
    pub fn control_before_initialization(root: &Path) -> Result<Self, Error> {
        Self::source_inner(root, DatabaseFile::Control, true)
    }
    fn source_inner(root: &Path, file: DatabaseFile, allow_empty: bool) -> Result<Self, Error> {
        directory(root)?;
        let path = root.join(file.name());
        let binding = Self {
            root: root.to_owned(),
            path: path.clone(),
            file,
            root_pin: pin(root, true)?,
            file_pin: pin(&path, false)?,
        };
        binding.check()?;
        if !allow_empty && binding.file_pin.metadata().map_err(io)?.len() == 0 {
            return Err(Error::Invalid);
        }
        Ok(binding)
    }
    pub fn target(root: &Path, source: &Self) -> Result<Self, Error> {
        use std::os::unix::fs::OpenOptionsExt;
        directory(root)?;
        if root.starts_with(&source.root)
            || source.root.starts_with(root)
            || fs::read_dir(root).map_err(io)?.next().is_some()
        {
            return Err(Error::UnsafeStorage);
        }
        let root_pin = pin(root, true)?;
        let path = root.join(source.file.name());
        let file_pin = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
            .open(&path)
            .map_err(io)?;
        let binding = Self {
            root: root.to_owned(),
            path,
            file: source.file,
            root_pin,
            file_pin,
        };
        binding.check()?;
        Ok(binding)
    }
    pub fn check(&self) -> Result<(), Error> {
        use std::os::unix::fs::MetadataExt;
        let root = directory(&self.root)?;
        let root_pin = self.root_pin.metadata().map_err(io)?;
        let file = regular(&self.path, root.uid())?;
        let pin = self.file_pin.metadata().map_err(io)?;
        if identity(&root) != identity(&root_pin)
            || identity(&file) != identity(&pin)
            || pin.nlink() != 1
            || root.uid() != pin.uid()
        {
            return Err(Error::UnsafeStorage);
        }
        for suffix in ["-wal", "-shm", "-journal"] {
            let path = self.root.join(format!("{}{suffix}", self.file.name()));
            match fs::symlink_metadata(&path) {
                Ok(_) => {
                    regular(&path, root.uid())?;
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err(Error::UnsafeStorage),
            }
        }
        Ok(())
    }
    pub fn same_origin(&self, other: &Self) -> Result<bool, Error> {
        self.check()?;
        other.check()?;
        let same = self.file == other.file
            && identity(&self.root_pin.metadata().map_err(io)?)
                == identity(&other.root_pin.metadata().map_err(io)?)
            && identity(&self.file_pin.metadata().map_err(io)?)
                == identity(&other.file_pin.metadata().map_err(io)?);
        self.check()?;
        other.check()?;
        Ok(same)
    }
    pub fn sync(&self) -> Result<(), Error> {
        self.check()?;
        self.file_pin.sync_all().map_err(io)?;
        self.root_pin.sync_all().map_err(io)?;
        self.check()
    }
    pub fn has_sidecars(&self) -> Result<bool, Error> {
        for suffix in ["-wal", "-shm", "-journal"] {
            match fs::symlink_metadata(self.root.join(format!("{}{suffix}", self.file.name()))) {
                Ok(_) => return Ok(true),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err(Error::UnsafeStorage),
            }
        }
        Ok(false)
    }
}

#[cfg(unix)]
fn identity(metadata: &fs::Metadata) -> (u64, u64) {
    use std::os::unix::fs::MetadataExt;
    (metadata.dev(), metadata.ino())
}
#[cfg(unix)]
fn directory(root: &Path) -> Result<fs::Metadata, Error> {
    use std::{os::unix::fs::MetadataExt, path::Component};
    if !root.is_absolute() {
        return Err(Error::UnsafeStorage);
    }
    let mut current = PathBuf::new();
    for component in root.components() {
        if !matches!(component, Component::RootDir | Component::Normal(_)) {
            return Err(Error::UnsafeStorage);
        }
        current.push(component);
        let item = fs::symlink_metadata(&current).map_err(io)?;
        if !item.is_dir() || item.file_type().is_symlink() {
            return Err(Error::UnsafeStorage);
        }
    }
    let metadata = fs::symlink_metadata(root).map_err(io)?;
    if metadata.mode() & 0o7777 != 0o700 {
        return Err(Error::UnsafeStorage);
    }
    Ok(metadata)
}
#[cfg(unix)]
fn regular(path: &Path, owner: u32) -> Result<fs::Metadata, Error> {
    use std::os::unix::fs::MetadataExt;
    let file = fs::symlink_metadata(path).map_err(io)?;
    if !file.is_file()
        || file.file_type().is_symlink()
        || file.nlink() != 1
        || file.uid() != owner
        || file.mode() & 0o7777 != 0o600
    {
        return Err(Error::UnsafeStorage);
    }
    Ok(file)
}
#[cfg(unix)]
fn pin(path: &Path, directory: bool) -> Result<File, Error> {
    use rustix::fs::{Mode, OFlags, open};
    let flags = OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK;
    open(
        path,
        flags
            | if directory {
                OFlags::DIRECTORY
            } else {
                OFlags::empty()
            },
        Mode::empty(),
    )
    .map(File::from)
    .map_err(|_| Error::UnsafeStorage)
}
fn io(_: std::io::Error) -> Error {
    Error::UnsafeStorage
}

#[cfg(not(unix))]
impl Binding {
    pub fn same_origin(&self, _: &Self) -> Result<bool, Error> {
        Err(Error::UnsafeStorage)
    }
    pub fn control_before_initialization(_: &Path) -> Result<Self, Error> {
        Err(Error::UnsafeStorage)
    }
    pub fn source(_: &Path, _: DatabaseFile) -> Result<Self, Error> {
        Err(Error::UnsafeStorage)
    }
    pub fn target(_: &Path, _: &Self) -> Result<Self, Error> {
        Err(Error::UnsafeStorage)
    }
    pub fn check(&self) -> Result<(), Error> {
        Err(Error::UnsafeStorage)
    }
    pub fn sync(&self) -> Result<(), Error> {
        Err(Error::UnsafeStorage)
    }
    pub fn has_sidecars(&self) -> Result<bool, Error> {
        Err(Error::UnsafeStorage)
    }
}
