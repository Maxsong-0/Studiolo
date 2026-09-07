#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;

pub fn private_dir() -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    directory
}
