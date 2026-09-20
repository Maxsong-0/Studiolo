//! Exact call inventories and boundary anchors for the final M6 implementation.
use std::path::Path;
pub(super) fn remainder(crate_name: &str, path: &Path, source: &str) -> String {
    if crate_name != "studiolo-local-server" {
        return source.to_owned();
    }
    if path.ends_with("crates/local-server/src/board/executor/socket.rs") {
        return super::authority_tokens::checked_closed(
            source,
            &[
                "OFlags::RDONLY|OFlags::DIRECTORY|OFlags::NOFOLLOW|OFlags::CLOEXEC",
                "FileType::from_raw_mode(stat.st_mode)!=FileType::Socket",
                "identity(&stat)!=self.identity",
                "ifself.released.swap(true,Ordering::SeqCst)",
                "Ok(stat)ifidentity(&stat)!=self.identity=>returnOk(())",
            ],
            &[
                ("fs::File", 1),
                ("UnixListener::bind(&path)", 1),
                ("statat(&parent,name,AtFlags::SYMLINK_NOFOLLOW)", 1),
                ("unlinkat(&self.parent,self.name(),AtFlags::empty())", 1),
                (
                    "statat(&self.parent,self.name(),AtFlags::SYMLINK_NOFOLLOW)",
                    2,
                ),
                (
                    "chmodat(&lease.parent,lease.name(),Mode::RUSR|Mode::WUSR,AtFlags::empty())",
                    1,
                ),
                (
                    "rustix::fs::{AtFlags,FileType,Mode,OFlags,Stat,chmodat,open,statat,unlinkat}",
                    1,
                ),
                (
                    "open(path.parent().ok_or(\"Runnersocket父目录缺失\")?,OFlags::RDONLY|OFlags::DIRECTORY|OFlags::NOFOLLOW|OFlags::CLOEXEC,Mode::empty())",
                    1,
                ),
                (
                    "File::from(open(path.parent().ok_or(\"Runnersocket父目录缺失\")?,OFlags::RDONLY|OFlags::DIRECTORY|OFlags::NOFOLLOW|OFlags::CLOEXEC,Mode::empty()).map_err(|_|\"Runnersocket父目录不可用\")?)",
                    1,
                ),
            ],
        );
    }
    if path.ends_with("crates/local-server/src/board/executor/landing_store.rs") {
        return super::authority_tokens::checked_closed(
            source,
            &[
                "super::config::private_directory(root)?;",
                "letpath=root.join(format!(\"landing-{identity}.lock\"));",
                "self.root.join(format!(\"landing-{key}-{phase}.json\"))",
                "FlockOperation::NonBlockingLockExclusive",
                "implDropforLease",
                "if(current.dev(),current.ino())!=(held.dev(),held.ino())",
                "m.nlink()!=1",
                "m.mode()&0o777!=0o600",
                "ifbytes.len()>131072",
            ],
            &[
                (".sync_all()", 2),
                ("File::from(fd)", 1),
                (".write_all(&bytes)", 1),
                ("File::open(&self.root)", 1),
                ("fs::symlink_metadata(&path)", 1),
                ("fs::{self,File,OpenOptions}", 1),
                ("fs::canonicalize(repository)", 1),
                ("fs::symlink_metadata(repository)", 1),
                ("os::unix::fs::{MetadataExt,OpenOptionsExt}", 1),
                ("rustix::fs::{FlockOperation,Mode,OFlags,flock,open}", 1),
                ("flock(&file,FlockOperation::NonBlockingLockExclusive)", 1),
                ("flock(&self._file,FlockOperation::Unlock)", 1),
                (
                    "OpenOptions::new().create_new(true).write(true).mode(0o600).open(self.path(key,phase))",
                    1,
                ),
                (
                    "open(self.path(key,phase),OFlags::RDONLY|OFlags::NOFOLLOW|OFlags::CLOEXEC,Mode::empty())",
                    1,
                ),
                (
                    "open(&path,OFlags::RDWR|OFlags::CREATE|OFlags::NOFOLLOW|OFlags::CLOEXEC,Mode::RUSR|Mode::WUSR)",
                    1,
                ),
                (
                    "open(repository,OFlags::RDONLY|OFlags::DIRECTORY|OFlags::NOFOLLOW|OFlags::CLOEXEC,Mode::empty())",
                    1,
                ),
                (
                    "File::from(open(repository,OFlags::RDONLY|OFlags::DIRECTORY|OFlags::NOFOLLOW|OFlags::CLOEXEC,Mode::empty()).map_err(|_|\"仓库不可用\")?)",
                    1,
                ),
                (
                    "File::from(open(&path,OFlags::RDWR|OFlags::CREATE|OFlags::NOFOLLOW|OFlags::CLOEXEC,Mode::RUSR|Mode::WUSR).map_err(|_|\"仓库落地锁不可用\")?)",
                    1,
                ),
            ],
        );
    }
    if path.ends_with("crates/local-server/src/board/executor/landing_candidate.rs") {
        return super::authority_tokens::checked_closed(
            source,
            &[
                "letbase=format!(\"sources/attempts/{task}/{}\",attempt.attempt_id);",
                "ifreceipt.manifest_path.as_str()!=format!(\"{base}/manifest.json\")||receipt.artifacts.len()!=2",
                "object.revision!=pointer.revision",
                "hash(&object.content)!=pointer.sha256",
                "metadata.retained_path!=retained",
                "ifbytes!=patch",
            ],
            &[
                ("std::fs::read(&retained)", 1),
                ("std::fs::symlink_metadata(&retained)", 1),
                (
                    "letbase=format!(\"sources/attempts/{task}/{}\",attempt.attempt_id);",
                    1,
                ),
            ],
        );
    }
    source.to_owned()
}
