//! 只读受信私有配置；该文件只有声明与socket引用，永不载入模型长期密钥。
use super::StartupError;
use std::{fs, io::Read, os::unix::fs::MetadataExt, path::Path};
use studiolo_local_server::board::RunnerConfig;
pub(super) fn load(path: &Path) -> Result<RunnerConfig, StartupError> {
    let deny = || StartupError::Configuration;
    let parent = path.parent().ok_or_else(deny)?;
    if !path.is_absolute() || fs::canonicalize(path).map_err(|_| deny())? != path {
        return Err(deny());
    }
    let before = fs::symlink_metadata(path).map_err(|_| deny())?;
    let directory = fs::symlink_metadata(parent).map_err(|_| deny())?;
    let uid = rustix::process::geteuid().as_raw();
    if !before.is_file()
        || before.nlink() != 1
        || before.uid() != uid
        || before.mode() & 0o777 != 0o600
        || before.len() > 131_072
        || !directory.is_dir()
        || directory.uid() != uid
        || directory.mode() & 0o777 != 0o700
    {
        return Err(deny());
    }
    let mut file = fs::File::open(path).map_err(|_| deny())?;
    let opened = file.metadata().map_err(|_| deny())?;
    if (before.dev(), before.ino()) != (opened.dev(), opened.ino()) {
        return Err(deny());
    }
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(131_073)
        .read_to_end(&mut bytes)
        .map_err(|_| deny())?;
    let after = fs::symlink_metadata(path).map_err(|_| deny())?;
    if bytes.len() > 131_072
        || (
            before.dev(),
            before.ino(),
            before.len(),
            before.mtime(),
            before.mtime_nsec(),
        ) != (
            after.dev(),
            after.ino(),
            after.len(),
            after.mtime(),
            after.mtime_nsec(),
        )
    {
        return Err(deny());
    }
    serde_json::from_slice(&bytes).map_err(|_| deny())
}
