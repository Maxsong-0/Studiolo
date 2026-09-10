//! Private no-link directory-handle primitives for the read-only adapter.

use std::path::Path;

use cap_std::fs::{Dir, File, Metadata, MetadataExt as _};
use studiolo_core::VaultPath;

#[cfg(not(unix))]
use super::limit;
use super::{private, unsafe_node};
use crate::BrokerError;

#[cfg(all(unix, not(target_os = "linux")))]
use rustix::fs::openat;
#[cfg(unix)]
use rustix::fs::{Mode, OFlags, open};

#[cfg(unix)]
pub(super) fn open_root(root: &Path) -> Result<Dir, BrokerError> {
    if root
        .components()
        .any(|component| component == std::path::Component::ParentDir)
    {
        return Err(BrokerError::io(
            "open read-only root",
            std::io::Error::other("parent traversal in selected root"),
        ));
    }
    // Remove a trailing slash or `/.` so neither can bypass final-component NOFOLLOW.
    let selected = root.components().collect::<std::path::PathBuf>();
    let handle = open(
        &selected,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::empty(),
    )
    .map_err(|error| BrokerError::io("open read-only root", error.into()))?;
    Ok(Dir::from_std_file(std::fs::File::from(handle)))
}

#[cfg(not(unix))]
pub(super) fn open_root(_root: &Path) -> Result<Dir, BrokerError> {
    Err(limit(
        "read-only rooted access is unsupported on this platform",
    ))
}

#[cfg(unix)]
pub(super) fn open_private_root(root: &Path) -> Result<Dir, BrokerError> {
    use std::path::Component;
    if !root.is_absolute()
        || root
            .components()
            .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(super::limit(
            "private read-only root requires a clean absolute path",
        ));
    }
    let mut directory = open_root(Path::new("/"))?;
    private::ancestor(&directory)?;
    for component in root.components() {
        let Component::Normal(name) = component else {
            continue;
        };
        let handle = rustix::fs::openat(
            &directory,
            name,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty(),
        )
        .map_err(|error| BrokerError::io("open private read-only ancestor", error.into()))?;
        directory = Dir::from_std_file(std::fs::File::from(handle));
        private::ancestor(&directory)?;
    }
    private::directory(&directory, true)?;
    Ok(directory)
}

#[cfg(not(unix))]
pub(super) fn open_private_root(_root: &Path) -> Result<Dir, BrokerError> {
    Err(limit("private read-only access requires Unix"))
}

pub(super) fn open_directory(
    root: &Dir,
    path: &VaultPath,
    private: bool,
) -> Result<Dir, BrokerError> {
    let (parent, name) = parent(root, path, private)?;
    child_directory(root, &parent, &name, path, private)
}

pub(super) fn open_file(root: &Dir, path: &VaultPath, private: bool) -> Result<File, BrokerError> {
    let (parent, name) = parent(root, path, private)?;
    collision(&parent, &name, path)?;
    scanned_file(root, &parent, &name, path, private)
}

// 调用方必须已完整枚举并检查同层名称；扫描器仍须执行路径重开与目录身份复核。
pub(super) fn scanned_file(
    root: &Dir,
    _parent: &Dir,
    _name: &str,
    path: &VaultPath,
    private: bool,
) -> Result<File, BrokerError> {
    private::directory(root, private)?;
    private::directory(_parent, private)?;
    #[cfg(target_os = "linux")]
    let handle = rustix::fs::openat2(
        root,
        path.as_str(),
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK,
        Mode::empty(),
        rustix::fs::ResolveFlags::BENEATH
            | rustix::fs::ResolveFlags::NO_MAGICLINKS
            | rustix::fs::ResolveFlags::NO_SYMLINKS
            | rustix::fs::ResolveFlags::NO_XDEV,
    );
    #[cfg(all(unix, not(target_os = "linux")))]
    let handle = openat(
        _parent,
        _name,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK,
        Mode::empty(),
    );
    #[cfg(unix)]
    {
        let handle = handle.map_err(|error| node_error(path, error.into()))?;
        let file = File::from_std(std::fs::File::from(handle));
        let metadata = file
            .metadata()
            .map_err(|error| BrokerError::io("stat opened read-only file", error))?;
        regular(path, &metadata)?;
        private::file(&metadata, private)?;
        same_device(root, &metadata, path)?;
        private::directory(root, private)?;
        private::directory(_parent, private)?;
        Ok(file)
    }
    #[cfg(not(unix))]
    Err(limit(
        "read-only rooted access is unsupported on this platform",
    ))
}

fn parent(root: &Dir, path: &VaultPath, private: bool) -> Result<(Dir, String), BrokerError> {
    private::directory(root, private)?;
    let mut directory = root
        .try_clone()
        .map_err(|error| BrokerError::io("clone read-only root", error))?;
    let mut components = path.as_str().split('/').peekable();
    let mut partial = String::new();
    while let Some(component) = components.next() {
        if components.peek().is_none() {
            return Ok((directory, component.to_owned()));
        }
        if !partial.is_empty() {
            partial.push('/');
        }
        partial.push_str(component);
        let logical =
            VaultPath::parse(&partial).map_err(|_| unsafe_node(path, "invalid parent path"))?;
        directory = child_directory(root, &directory, component, &logical, private)?;
    }
    Err(unsafe_node(path, "missing file name"))
}

fn child_directory(
    root: &Dir,
    parent: &Dir,
    name: &str,
    path: &VaultPath,
    private: bool,
) -> Result<Dir, BrokerError> {
    collision(parent, name, path)?;
    scanned_directory(root, parent, name, path, private)
}

// 跳过重复名称枚举，不跳过 rooted/no-follow/no-xdev 与设备验证。
pub(super) fn scanned_directory(
    root: &Dir,
    _parent: &Dir,
    _name: &str,
    path: &VaultPath,
    private: bool,
) -> Result<Dir, BrokerError> {
    private::directory(root, private)?;
    private::directory(_parent, private)?;
    #[cfg(target_os = "linux")]
    let handle = rustix::fs::openat2(
        root,
        path.as_str(),
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::empty(),
        rustix::fs::ResolveFlags::BENEATH
            | rustix::fs::ResolveFlags::NO_MAGICLINKS
            | rustix::fs::ResolveFlags::NO_SYMLINKS
            | rustix::fs::ResolveFlags::NO_XDEV,
    );
    #[cfg(all(unix, not(target_os = "linux")))]
    let handle = openat(
        _parent,
        _name,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::empty(),
    );
    #[cfg(unix)]
    {
        let handle = handle.map_err(|error| node_error(path, error.into()))?;
        let directory = Dir::from_std_file(std::fs::File::from(handle));
        let metadata = directory
            .dir_metadata()
            .map_err(|error| BrokerError::io("stat read-only directory", error))?;
        same_device(root, &metadata, path)?;
        private::directory(&directory, private)?;
        private::directory(root, private)?;
        private::directory(_parent, private)?;
        Ok(directory)
    }
    #[cfg(not(unix))]
    Err(limit(
        "read-only rooted access is unsupported on this platform",
    ))
}

fn collision(directory: &Dir, requested: &str, path: &VaultPath) -> Result<(), BrokerError> {
    let requested_key = requested.to_lowercase();
    for (index, entry) in directory
        .entries()
        .map_err(|error| BrokerError::io("inspect read-only names", error))?
        .enumerate()
    {
        if index >= 50_000 {
            return Err(unsafe_node(
                path,
                "directory exceeds safe name lookup limit",
            ));
        }
        let name = entry
            .map_err(|error| BrokerError::io("inspect read-only name", error))?
            .file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if name != requested && name.to_lowercase() == requested_key {
            return Err(unsafe_node(path, "case-colliding filesystem name"));
        }
    }
    Ok(())
}

fn node_error(path: &VaultPath, error: std::io::Error) -> BrokerError {
    if error.kind() == std::io::ErrorKind::NotFound {
        BrokerError::NotFound(path.clone())
    } else {
        unsafe_node(path, "no-link rooted resolution failed")
    }
}

pub(super) fn same_device(
    root: &Dir,
    metadata: &Metadata,
    path: &VaultPath,
) -> Result<(), BrokerError> {
    #[cfg(unix)]
    if root
        .dir_metadata()
        .map_err(|error| BrokerError::io("stat read-only root", error))?
        .dev()
        != metadata.dev()
    {
        return Err(unsafe_node(path, "cross-device entry is not readable"));
    }
    Ok(())
}

pub(super) fn regular(path: &VaultPath, metadata: &Metadata) -> Result<(), BrokerError> {
    if !metadata.is_file() {
        return Err(unsafe_node(path, "not a regular file"));
    }
    #[cfg(unix)]
    if metadata.nlink() != 1 {
        return Err(unsafe_node(path, "hard-linked or unlinked file"));
    }
    Ok(())
}

pub(super) fn unchanged(before: &Metadata, after: &Metadata) -> bool {
    if before.len() != after.len() || before.modified().ok() != after.modified().ok() {
        return false;
    }
    #[cfg(unix)]
    if before.dev() != after.dev()
        || before.ino() != after.ino()
        || before.ctime() != after.ctime()
        || before.ctime_nsec() != after.ctime_nsec()
    {
        return false;
    }
    true
}
