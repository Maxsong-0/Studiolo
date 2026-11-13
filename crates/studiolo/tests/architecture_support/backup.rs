//! BackupWorker 只写自己登记的固定备份状态与清单；协调器不持有任何 owner 的数据库句柄。

use std::path::Path;

pub(super) fn fixed_checkpoint_files(source: &str) -> bool {
    let compact = source.split_whitespace().collect::<String>();
    compact.contains("pubfnname(self)->&'staticstr{matchself{Self::Auth=>\"auth.sqlite3\",Self::Audit=>\"audit.sqlite3\",Self::Control=>\"control.sqlite3\",Self::Bridge=>\"bridge.sqlite3\",Self::Agent=>\"agent.sqlite3\",}}")
}

pub(super) fn remainder(crate_name: &str, path: &Path, source: &str) -> String {
    if crate_name == "studiolo-checkpoint-storage" {
        return sqlite_checkpoint(path, source);
    }
    if crate_name == "studiolo-audit-ledger" {
        return operation_gate(path, source);
    }
    if crate_name != "studiolo-backup" || !path.ends_with("crates/backup/src/store/storage.rs") {
        return source.to_owned();
    }
    let rest = source.split_whitespace().collect::<String>();
    // 固定文件名必须在场，换掉文件名就拿不到豁免。
    for required in [
        "\"backup-state.json\"",
        "\"backup-state.json.tmp\"",
        "\"manifest.json\"",
        "\"manifest.json.tmp\"",
    ] {
        if !rest.contains(required) {
            return rest;
        }
    }
    rest.replace(
        "userustix::fs::{AtFlags,FileType,Mode,OFlags,fstat,fsync,open,openat,renameat,unlinkat};",
        "",
    )
}

fn operation_gate(path: &Path, source: &str) -> String {
    use super::authority_tokens::checked_closed;
    if path.ends_with("crates/audit-ledger/src/operation.rs") {
        return checked_closed(
            source,
            &["implDropforLease"],
            &[
                ("std::fs::File", 1),
                (
                    "rustix::fs::flock(&self.0,rustix::fs::FlockOperation::Unlock)",
                    1,
                ),
            ],
        );
    }
    if path.ends_with("crates/audit-ledger/src/sqlite/storage/operation.rs") {
        return checked_closed(
            source,
            &[
                "self.root.join(\"checkpoint-operations.lock\")",
                "letflags=OFlags::RDWR|OFlags::CLOEXEC|OFlags::NOFOLLOW;",
                "super::inspect_file(&path,self.owner,false)?",
                "identity.is_some_and(|expected|expected!=actual)",
                "found.len()!=0||(found.dev(),found.ino())!=(pinned.dev(),pinned.ino())",
                "FlockOperation::NonBlockingLockExclusive",
                "FlockOperation::NonBlockingLockShared",
                "ifInstant::now()>=deadline",
            ],
            &[
                ("rustix::fs::{FlockOperation,Mode,OFlags,flock,open}", 1),
                ("fs::File", 1),
                ("os::unix::fs::MetadataExt", 1),
                (
                    "open(&path,flags|OFlags::CREATE|OFlags::EXCL,Mode::RUSR|Mode::WUSR)",
                    1,
                ),
                ("File::from(fd)", 1),
                (
                    "File::from(open(&path,flags,Mode::empty()).map_err(|_|AuditError::UnsafeStorage)?)",
                    1,
                ),
                ("file.sync_all()", 1),
                ("File::open(&self.root)", 1),
                ("dir.sync_all()", 1),
                ("flock(&file,operation)", 1),
            ],
        );
    }
    source.to_owned()
}

// 只给单库封存适配器精确的固定路径 IO；普通协调器仍不可依赖本模块。
fn sqlite_checkpoint(path: &Path, source: &str) -> String {
    use super::authority_tokens::checked_closed;
    if path.ends_with("crates/checkpoint-storage/src/sqlite.rs") {
        return checked_closed(
            source,
            &[
                "binding.check()?;",
                "connection.busy_timeout(Duration::ZERO)",
                "connection.progress_handler(1000,Some(move||Instant::now()>=deadline))",
                "reserve.execute_batch(\"BEGINIMMEDIATE\")",
                "source=connect(&binding,true,deadline)?",
            ],
            &[(
                "Connection::open_with_flags(&binding.path,access|OpenFlags::SQLITE_OPEN_NO_MUTEX|OpenFlags::SQLITE_OPEN_NOFOLLOW|OpenFlags::SQLITE_OPEN_PRIVATE_CACHE)",
                1,
            )],
        );
    }
    if path.ends_with("crates/checkpoint-storage/src/paths.rs") {
        return checked_closed(
            source,
            &[
                "Self::source_inner(root,file,false)",
                "Self::source_inner(root,DatabaseFile::Control,true)",
                "if!allow_empty&&binding.file_pin.metadata().map_err(io)?.len()==0",
                "root.join(file.name())",
                "root.join(source.file.name())",
                "fs::read_dir(root).map_err(io)?.next().is_some()",
                "identity(&file)!=identity(&pin)",
                "pin.nlink()!=1",
                "file.mode()&0o7777!=0o600",
                "metadata.mode()&0o7777!=0o700",
                "item.file_type().is_symlink()",
                "letflags=OFlags::RDONLY|OFlags::CLOEXEC|OFlags::NOFOLLOW|OFlags::NONBLOCK;",
            ],
            &[
                ("fs::{self,File,OpenOptions}", 1),
                ("os::unix::fs::OpenOptionsExt", 1),
                ("os::unix::fs::MetadataExt", 4),
                ("&fs::Metadata", 1),
                ("Result<fs::Metadata,Error>", 2),
                ("fs::read_dir(root)", 1),
                ("fs::symlink_metadata(&path)", 1),
                (
                    "fs::symlink_metadata(self.root.join(format!(\"{}{suffix}\",self.file.name())))",
                    1,
                ),
                ("fs::symlink_metadata(&current)", 1),
                ("fs::symlink_metadata(root)", 1),
                ("fs::symlink_metadata(path)", 1),
                ("self.file_pin.sync_all()", 1),
                ("self.root_pin.sync_all()", 1),
                (
                    "OpenOptions::new().read(true).write(true).create_new(true).mode(0o600).custom_flags(rustix::fs::OFlags::NOFOLLOW.bits()asi32).open(&path)",
                    1,
                ),
                ("rustix::fs::{Mode,OFlags,open}", 1),
                (
                    "open(path,flags|ifdirectory{OFlags::DIRECTORY}else{OFlags::empty()},Mode::empty())",
                    1,
                ),
            ],
        );
    }
    source.to_owned()
}
