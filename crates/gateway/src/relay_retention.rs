//! 对仅供 Gateway 使用的原始暂存区（raw spool）做独立的硬 TTL 清理。
use crate::{Clock, relay_types::RelayError};
use std::{
    fs,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

pub(crate) struct Retention(Arc<AtomicBool>);
impl Retention {
    pub(crate) fn start(
        root: std::path::PathBuf,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, RelayError> {
        sweep(&root, clock.now_unix_seconds())?;
        let stop = Arc::new(AtomicBool::new(false));
        let signal = stop.clone();
        thread::spawn(move || {
            while !signal.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_secs(30));
                let _ = sweep(&root, clock.now_unix_seconds());
            }
        });
        Ok(Self(stop))
    }
}
impl Drop for Retention {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}
pub(crate) fn sweep(root: &Path, now: i64) -> Result<(), RelayError> {
    for entry in fs::read_dir(root).map_err(|_| RelayError::Storage)? {
        let entry = entry.map_err(|_| RelayError::Storage)?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            return Err(RelayError::Storage);
        };
        let Some(prefix) = name.strip_suffix(".raw") else {
            continue;
        };
        let Some((_, deadline)) = prefix.rsplit_once('-') else {
            return Err(RelayError::Storage);
        };
        let deadline: i64 = deadline.parse().map_err(|_| RelayError::Storage)?;
        let metadata = fs::symlink_metadata(entry.path()).map_err(|_| RelayError::Storage)?;
        if !metadata.is_file() {
            return Err(RelayError::Storage);
        }
        if now >= deadline {
            fs::remove_file(entry.path()).map_err(|_| RelayError::Storage)?;
        }
    }
    Ok(())
}
pub(crate) fn available(root: &Path, response_limit: u32, now: i64) -> Result<(), RelayError> {
    sweep(root, now)?;
    let mut total = 0u64;
    for entry in fs::read_dir(root).map_err(|_| RelayError::Storage)? {
        total = total.saturating_add(
            entry
                .map_err(|_| RelayError::Storage)?
                .metadata()
                .map_err(|_| RelayError::Storage)?
                .len(),
        );
    }
    if total.saturating_add(u64::from(response_limit)) > 128 * 1024 * 1024 {
        return Err(RelayError::Storage);
    }
    Ok(())
}
