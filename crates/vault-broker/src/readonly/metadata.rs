//! 附件只取元信息：Linux 使用 O_PATH 句柄，其他 Unix 使用父句柄下的不跟随 stat。

use cap_std::fs::{Dir, Metadata};
use studiolo_core::VaultPath;

#[cfg(any(target_os = "linux", not(unix)))]
use super::unsafe_node;
use super::{private, rooted};
use crate::BrokerError;

pub(super) fn stable_file(
    root: &Dir,
    parent: &Dir,
    name: &str,
    path: &VaultPath,
    private: bool,
) -> Result<u64, BrokerError> {
    let before = inspect(root, parent, name, path, private)?;
    let after = inspect(root, parent, name, path, private)?;
    if !rooted::unchanged(&before, &after) {
        return Err(BrokerError::UnstableRead(path.clone()));
    }
    Ok(after.len())
}

fn inspect(
    root: &Dir,
    _parent: &Dir,
    _name: &str,
    path: &VaultPath,
    private: bool,
) -> Result<Metadata, BrokerError> {
    private::directory(root, private)?;
    private::directory(_parent, private)?;
    #[cfg(target_os = "linux")]
    let metadata = {
        use rustix::fs::{Mode, OFlags, ResolveFlags, openat2};
        let handle = openat2(
            root,
            path.as_str(),
            OFlags::PATH | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
            ResolveFlags::BENEATH
                | ResolveFlags::NO_MAGICLINKS
                | ResolveFlags::NO_SYMLINKS
                | ResolveFlags::NO_XDEV,
        )
        .map_err(|_| unsafe_node(path, "no-link rooted metadata resolution failed"))?;
        let file = cap_std::fs::File::from_std(std::fs::File::from(handle));
        file.metadata()
            .map_err(|error| BrokerError::io("stat metadata-only file", error))?
    };
    #[cfg(all(unix, not(target_os = "linux")))]
    let metadata = _parent
        .symlink_metadata(_name)
        .map_err(|error| BrokerError::io("stat metadata-only file", error))?;
    #[cfg(unix)]
    {
        rooted::regular(path, &metadata)?;
        private::file(&metadata, private)?;
        rooted::same_device(root, &metadata, path)?;
        private::directory(root, private)?;
        private::directory(_parent, private)?;
        Ok(metadata)
    }
    #[cfg(not(unix))]
    Err(unsafe_node(
        path,
        "metadata-only rooted access is unsupported on this platform",
    ))
}
