//! 固定私有准入文件；锁只活在 FD 上，崩溃后不留下需人工清除的状态。
use super::Storage;
use crate::{AuditError, operation::Lease};
use std::time::Instant;

#[cfg(unix)]
impl Storage {
    pub(in crate::sqlite) fn operation_lease(
        &self,
        exclusive: bool,
        deadline: Instant,
    ) -> Result<Lease, AuditError> {
        use rustix::fs::{FlockOperation, Mode, OFlags, flock, open};
        use std::{fs::File, os::unix::fs::MetadataExt, thread, time::Duration};
        self.check()?;
        let path = self.root.join("checkpoint-operations.lock");
        let flags = OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOFOLLOW;
        let (file, created) = match open(
            &path,
            flags | OFlags::CREATE | OFlags::EXCL,
            Mode::RUSR | Mode::WUSR,
        ) {
            Ok(fd) => (File::from(fd), true),
            Err(rustix::io::Errno::EXIST) => (
                File::from(
                    open(&path, flags, Mode::empty()).map_err(|_| AuditError::UnsafeStorage)?,
                ),
                false,
            ),
            Err(_) => return Err(AuditError::UnsafeStorage),
        };
        let pinned = file.metadata().map_err(|_| AuditError::UnsafeStorage)?;
        {
            let mut identity = self
                .operation_identity
                .lock()
                .map_err(|_| AuditError::Unavailable)?;
            let actual = (pinned.dev(), pinned.ino());
            if identity.is_some_and(|expected| expected != actual) {
                return Err(AuditError::UnsafeStorage);
            }
            *identity = Some(actual);
        }
        let inspect = || {
            self.check()?;
            let found =
                super::inspect_file(&path, self.owner, false)?.ok_or(AuditError::UnsafeStorage)?;
            if found.len() != 0 || (found.dev(), found.ino()) != (pinned.dev(), pinned.ino()) {
                return Err(AuditError::UnsafeStorage);
            }
            Ok(())
        };
        inspect()?;
        if created {
            file.sync_all().map_err(|_| AuditError::Unavailable)?;
            File::open(&self.root)
                .and_then(|dir| dir.sync_all())
                .map_err(|_| AuditError::Unavailable)?;
        }
        let operation = if exclusive {
            FlockOperation::NonBlockingLockExclusive
        } else {
            FlockOperation::NonBlockingLockShared
        };
        loop {
            if Instant::now() >= deadline {
                return Err(AuditError::Unavailable);
            }
            match flock(&file, operation) {
                Ok(()) => {
                    let lease = Lease(file);
                    inspect()?;
                    if Instant::now() >= deadline {
                        return Err(AuditError::Unavailable);
                    }
                    return Ok(lease);
                }
                Err(rustix::io::Errno::WOULDBLOCK | rustix::io::Errno::INTR) => {}
                Err(_) => return Err(AuditError::Unavailable),
            }
            thread::sleep(
                deadline
                    .saturating_duration_since(Instant::now())
                    .min(Duration::from_millis(5)),
            );
        }
    }
}

#[cfg(not(unix))]
impl Storage {
    pub(in crate::sqlite) fn operation_lease(
        &self,
        _: bool,
        _: Instant,
    ) -> Result<Lease, AuditError> {
        Err(AuditError::UnsupportedPlatform)
    }
}
