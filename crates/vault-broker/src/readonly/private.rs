//! 私有只读模式的属主与权限检查；全部检查已打开句柄的元信息。

#[cfg(unix)]
use cap_std::fs::MetadataExt as _;
use cap_std::fs::{Dir, Metadata};

use super::limit;
use crate::BrokerError;

#[cfg(unix)]
fn owner() -> Result<u32, BrokerError> {
    let uid = rustix::process::getuid();
    if uid != rustix::process::geteuid() {
        return Err(limit(
            "private read-only access rejects an elevated identity",
        ));
    }
    Ok(uid.as_raw())
}

pub(super) fn ancestor(directory: &Dir) -> Result<(), BrokerError> {
    #[cfg(unix)]
    {
        let metadata = directory
            .dir_metadata()
            .map_err(|error| BrokerError::io("stat private read-only ancestor", error))?;
        if metadata.uid() != 0 && metadata.uid() != owner()? {
            return Err(limit("private read-only ancestor has an unexpected owner"));
        }
        Ok(())
    }
    #[cfg(not(unix))]
    Err(limit("private read-only access requires Unix"))
}

pub(super) fn directory(directory: &Dir, enabled: bool) -> Result<(), BrokerError> {
    if !enabled {
        return Ok(());
    }
    let metadata = directory
        .dir_metadata()
        .map_err(|error| BrokerError::io("stat private read-only directory", error))?;
    check(&metadata, 0o700)
}

pub(super) fn file(metadata: &Metadata, enabled: bool) -> Result<(), BrokerError> {
    if !enabled {
        return Ok(());
    }
    check(metadata, 0o600)
}

fn check(metadata: &Metadata, mode: u32) -> Result<(), BrokerError> {
    #[cfg(unix)]
    {
        if metadata.uid() != owner()? || metadata.mode() & 0o7777 != mode {
            return Err(limit(
                "private read-only node has unsafe ownership or permissions",
            ));
        }
        Ok(())
    }
    #[cfg(not(unix))]
    Err(limit("private read-only access requires Unix"))
}
