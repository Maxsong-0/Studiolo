//! 独立 worker 的固定持久状态与清单；与原根绑定的独占租约阻止第二个写者。
//!
//! 只在一个已存在的私有目录（0700、属调用者）里操作四个状态文件及 backup.lock：
//! `backup-state.json`、`backup-state.json.tmp`、`manifest.json`、`manifest.json.tmp`。
//! 目录句柄在 `open` 时一次性取得并持有，之后所有操作都相对该句柄，
//! 不再解析路径；不创建目录、不删除目录、不执行任何外部进程。
//! 写入一律「固定临时名 → 写满 → fsync → 原地改名 → fsync 目录」，
//! 空间不足只留下被清理的临时名，既有内容不受影响。

use super::model::{BackupStateStore, PersistedState, StoreError};
use crate::checkpoint::manifest::CheckpointManifest;
use std::path::Path;
mod encoding;
use encoding::encode;
#[cfg(unix)]
mod lease;
#[cfg(unix)]
mod paths;
#[cfg(unix)]
use paths::{caller, check_directory, check_regular, walk};

/// 持久状态的固定文件名。
pub const STATE_FILE: &str = "backup-state.json";
/// 持久状态写入时的固定临时名。
pub const STATE_TEMP_FILE: &str = "backup-state.json.tmp";
/// 最终清单的固定文件名。
pub const MANIFEST_FILE: &str = "manifest.json";
/// 最终清单写入时的固定临时名。
pub const MANIFEST_TEMP_FILE: &str = "manifest.json.tmp";

/// 持久状态的字节上限。
pub const STATE_MAX_BYTES: u64 = 1024 * 1024;
/// 最终清单的字节上限。
pub const MANIFEST_MAX_BYTES: u64 = 4 * 1024 * 1024;

#[cfg(unix)]
use rustix::fs::{AtFlags, Mode, OFlags, fsync, openat, renameat, unlinkat};

/// 固定私有根目录上的备份状态存储。
pub struct BackupStore {
    #[cfg(unix)]
    root: rustix::fd::OwnedFd,
    #[cfg(unix)]
    lease: lease::Lease,
    _single_writer: std::marker::PhantomData<std::cell::Cell<()>>,
}

#[cfg(unix)]
impl BackupStore {
    /// 打开已有存储，或显式初始化一个新的。
    ///
    /// 目录必须已经存在、属当前有效用户且权限为 0700；本函数不创建目录。
    pub fn open(root: &Path, initialize: bool) -> Result<Self, StoreError> {
        let owner = caller()?;
        let root_path = root;
        let root = walk(root_path)?;
        check_directory(&root, owner)?;
        if initialize {
            // 建锁之前先只读核一遍：建好过或有残留就在这里拒绝，不留下一个没有状态文件的空锁。
            // 显式初始化不接管旧锁，留下的空锁只能人工清（见 docs/contracts/backup-worker-store.md）。
            refuse_initialized(&root, owner)?;
        }
        let lease = lease::Lease::acquire(root_path, &root, initialize)?;
        let store = Self {
            root,
            lease,
            _single_writer: std::marker::PhantomData,
        };
        if initialize {
            // 建锁之前那一遍没有锁保护，拿到锁再核一遍才算数。
            refuse_initialized(&store.root, owner)?;
            store.replace(
                STATE_FILE,
                STATE_TEMP_FILE,
                &encode(&PersistedState::initial(), STATE_MAX_BYTES)?,
            )?;
        } else if !present(&store.root, STATE_FILE, STATE_MAX_BYTES, owner)? {
            return Err(StoreError::MissingStore);
        }
        Ok(store)
    }

    fn read(&self, name: &str, max: u64) -> Result<Option<Vec<u8>>, StoreError> {
        let owner = caller()?;
        self.lease.check(&self.root)?;
        check_directory(&self.root, owner)?;
        let flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC;
        let file = match openat(&self.root, name, flags, Mode::empty()) {
            Ok(file) => file,
            Err(rustix::io::Errno::NOENT) => return Ok(None),
            Err(_) => return Err(StoreError::UnsafeStorage),
        };
        let length = check_regular(&file, owner, max)?;
        let mut bytes = Vec::new();
        let mut chunk = [0_u8; 64 * 1024];
        loop {
            match rustix::io::read(&file, &mut chunk) {
                Ok(0) => break,
                Ok(count) => {
                    if bytes.len().saturating_add(count) as u64 > max {
                        return Err(StoreError::LimitExceeded);
                    }
                    bytes.extend_from_slice(&chunk[..count]);
                }
                Err(rustix::io::Errno::INTR) => {}
                Err(_) => return Err(StoreError::Unavailable),
            }
        }
        if bytes.len() as u64 != length {
            return Err(StoreError::CorruptStore);
        }
        self.lease.check(&self.root)?;
        Ok(Some(bytes))
    }

    /// 固定临时名 → 写满 → fsync → 原地改名 → fsync 目录。
    fn replace(&self, name: &str, temporary: &str, bytes: &[u8]) -> Result<(), StoreError> {
        let owner = caller()?;
        self.lease.check(&self.root)?;
        check_directory(&self.root, owner)?;
        match unlinkat(&self.root, temporary, AtFlags::empty()) {
            Ok(()) | Err(rustix::io::Errno::NOENT) => {}
            Err(_) => return Err(StoreError::UnsafeStorage),
        }
        let flags =
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        let file = openat(&self.root, temporary, flags, Mode::RUSR | Mode::WUSR)
            .map_err(|_| StoreError::UnsafeStorage)?;
        if let Err(error) = self.fill(&file, bytes) {
            let _ = unlinkat(&self.root, temporary, AtFlags::empty());
            return Err(error);
        }
        if fsync(&file).is_err() {
            let _ = unlinkat(&self.root, temporary, AtFlags::empty());
            return Err(StoreError::Unavailable);
        }
        check_regular(&file, owner, u64::MAX)?;
        if renameat(&self.root, temporary, &self.root, name).is_err() {
            let _ = unlinkat(&self.root, temporary, AtFlags::empty());
            return Err(StoreError::Unavailable);
        }
        fsync(&self.root).map_err(|_| StoreError::Unavailable)?;
        self.lease.check(&self.root)
    }

    fn fill(&self, file: &rustix::fd::OwnedFd, mut bytes: &[u8]) -> Result<(), StoreError> {
        while !bytes.is_empty() {
            match rustix::io::write(file, bytes) {
                Ok(0) => return Err(StoreError::Unavailable),
                Ok(count) => bytes = &bytes[count..],
                Err(rustix::io::Errno::INTR) => {}
                Err(rustix::io::Errno::NOSPC) | Err(rustix::io::Errno::DQUOT) => {
                    return Err(StoreError::OutOfSpace);
                }
                Err(_) => return Err(StoreError::Unavailable),
            }
        }
        Ok(())
    }
}

/// 状态文件、它的临时名或清单任一已在：这个目录建过存储，或上次初始化留下了残留，都不能再初始化。
#[cfg(unix)]
fn refuse_initialized(root: &rustix::fd::OwnedFd, owner: u32) -> Result<(), StoreError> {
    if present(root, STATE_FILE, STATE_MAX_BYTES, owner)? {
        return Err(StoreError::AlreadyExists);
    }
    for name in [STATE_TEMP_FILE, MANIFEST_FILE, MANIFEST_TEMP_FILE] {
        if present(root, name, MANIFEST_MAX_BYTES, owner)? {
            return Err(StoreError::AlreadyExists);
        }
    }
    Ok(())
}

/// 固定名字的普通文件在不在；在就核属主、权限和大小，是链接或别的类型一律当不安全。
#[cfg(unix)]
fn present(
    root: &rustix::fd::OwnedFd,
    name: &str,
    max: u64,
    owner: u32,
) -> Result<bool, StoreError> {
    let flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC;
    match openat(root, name, flags, Mode::empty()) {
        Ok(file) => {
            check_regular(&file, owner, max)?;
            Ok(true)
        }
        Err(rustix::io::Errno::NOENT) => Ok(false),
        Err(_) => Err(StoreError::UnsafeStorage),
    }
}

#[cfg(unix)]
impl BackupStateStore for BackupStore {
    fn load_state(&self) -> Result<Option<PersistedState>, StoreError> {
        let Some(bytes) = self.read(STATE_FILE, STATE_MAX_BYTES)? else {
            return Ok(None);
        };
        let state: PersistedState =
            serde_json::from_slice(&bytes).map_err(|_| StoreError::CorruptStore)?;
        state.validate()?;
        Ok(Some(state))
    }

    fn save_state(&self, state: &PersistedState) -> Result<(), StoreError> {
        state.validate()?;
        let bytes = encode(state, STATE_MAX_BYTES)?;
        if bytes.len() as u64 > STATE_MAX_BYTES {
            return Err(StoreError::LimitExceeded);
        }
        self.replace(STATE_FILE, STATE_TEMP_FILE, &bytes)
    }

    fn commit_final_manifest(&self, manifest: &CheckpointManifest) -> Result<(), StoreError> {
        manifest.validate().map_err(|_| StoreError::CorruptStore)?;
        let bytes = encode(manifest, MANIFEST_MAX_BYTES)?;
        if bytes.len() as u64 > MANIFEST_MAX_BYTES {
            return Err(StoreError::LimitExceeded);
        }
        self.replace(MANIFEST_FILE, MANIFEST_TEMP_FILE, &bytes)
    }

    fn load_final_manifest(&self) -> Result<Option<CheckpointManifest>, StoreError> {
        let Some(bytes) = self.read(MANIFEST_FILE, MANIFEST_MAX_BYTES)? else {
            return Ok(None);
        };
        let manifest: CheckpointManifest =
            serde_json::from_slice(&bytes).map_err(|_| StoreError::CorruptStore)?;
        manifest.validate().map_err(|_| StoreError::CorruptStore)?;
        Ok(Some(manifest))
    }
}

#[cfg(not(unix))]
impl BackupStore {
    /// 非 Unix 平台没有实现安全的私有文件约束。
    pub fn open(_root: &Path, _initialize: bool) -> Result<Self, StoreError> {
        Err(StoreError::UnsupportedPlatform)
    }
}

#[cfg(not(unix))]
impl BackupStateStore for BackupStore {
    fn load_state(&self) -> Result<Option<PersistedState>, StoreError> {
        Err(StoreError::UnsupportedPlatform)
    }

    fn save_state(&self, _state: &PersistedState) -> Result<(), StoreError> {
        Err(StoreError::UnsupportedPlatform)
    }

    fn commit_final_manifest(&self, _manifest: &CheckpointManifest) -> Result<(), StoreError> {
        Err(StoreError::UnsupportedPlatform)
    }

    fn load_final_manifest(&self) -> Result<Option<CheckpointManifest>, StoreError> {
        Err(StoreError::UnsupportedPlatform)
    }
}
