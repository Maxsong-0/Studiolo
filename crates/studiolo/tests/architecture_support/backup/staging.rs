//! 新副本封存能力逐调用登记；任何额外任意路径写入仍由总守卫拒绝。
use super::super::authority_tokens::checked_closed;
use std::path::Path;
pub(super) fn remainder(path: &Path, source: &str) -> String {
    if path.ends_with("crates/backup/src/checkpoint/staging/io/cleanup.rs") {
        return checked_closed(
            source,
            &[
                "self.cleanup_binding(parent,name,reader)?;",
                "Some(id)==reader&&id.producer_uid==rustix::process::geteuid().as_raw()",
                "parent.access.require_writer()?;",
                "letmutremaining=MAX_NODES;",
                "ifdepth>64",
                "child.cleanup_binding(self,name,reader)?;",
                "st.st_uid!=rustix::process::geteuid().as_raw()",
                "(st.st_dev,st.st_ino)!=self.pin",
                "crate::checkpoint::staging::check_deadline(deadline)?;",
                "super::acl::require_no_acl(&self.fd,true)",
            ],
            &[
                ("fs::unlinkat(&parent.fd,name,fs::AtFlags::REMOVEDIR)", 1),
                ("fs::fchmod(&self.fd,Mode::RWXU)", 1),
                ("fs::statat(&self.fd,name,fs::AtFlags::SYMLINK_NOFOLLOW)", 1),
                (
                    "fs::statat(&parent.fd,name,fs::AtFlags::SYMLINK_NOFOLLOW)",
                    1,
                ),
                ("fs::fstat(&self.fd)", 1),
                ("fs::fstat(&fd)", 2),
                ("fs::openat(&self.fd,name,super::FLAGS,Mode::empty())", 2),
                ("fs::unlinkat(&self.fd,name,fs::AtFlags::REMOVEDIR)", 1),
                ("fs::unlinkat(&self.fd,name,fs::AtFlags::empty())", 1),
            ],
        );
    }
    if path.ends_with("crates/backup/src/checkpoint/staging/io/lease.rs") {
        return checked_closed(
            source,
            &[
                "NAME:&str=\".producer-lease\"",
                "OFlags::RDWR|OFlags::NOFOLLOW|OFlags::CLOEXEC|OFlags::NONBLOCK",
                "st.st_mode&0o7777!=0o600",
                "st.st_nlink!=1||st.st_size!=0",
                "(current.st_dev,current.st_ino)!=original",
                "acl::require_no_acl(fd,false)?;",
            ],
            &[
                (
                    "fs::openat(&parent.fd,NAME,flags|OFlags::CREATE|OFlags::EXCL,Mode::RUSR|Mode::WUSR)",
                    1,
                ),
                ("fs::openat(&parent.fd,NAME,flags,Mode::empty())", 2),
                ("fs::fsync(&fd)", 1),
                (
                    "fs::flock(&fd,fs::FlockOperation::NonBlockingLockExclusive)",
                    1,
                ),
                (
                    "fs::statat(&parent.fd,NAME,fs::AtFlags::SYMLINK_NOFOLLOW)",
                    1,
                ),
                ("fs::fstat(fd)", 1),
            ],
        );
    }
    if path.ends_with("crates/backup/src/checkpoint/staging/io/acl.rs") {
        return checked_closed(
            source,
            &[
                "[\"system.posix_acl_access\",\"system.posix_acl_default\"]",
                "_=>returnErr(StagingError::UnsafeRoot)",
            ],
            &[("rustix::fs::fgetxattr(fd,name,&mutbyte[..])", 1)],
        );
    }
    if path.ends_with("crates/backup/src/checkpoint/staging/io/share.rs") {
        return checked_closed(
            source,
            &[
                "self.access!=Access::Private||depth>64||identity.producer_uid!=rustix::process::geteuid().as_raw()",
                "file.check(self,name)?;",
                "self.require_readonly()?;",
                "self.access=Access::SharingRoot(identity);",
                "letAccess::SharingRoot(identity)=self.accesselse",
            ],
            &[
                (
                    "fs::fchown(&self.fd,None,Some(rustix::process::Gid::from_raw(identity.reader_gid)))",
                    1,
                ),
                (
                    "fs::fchmod(&self.fd,Mode::RUSR|Mode::XUSR|Mode::RGRP|Mode::XGRP)",
                    1,
                ),
            ],
        );
    }
    if path.ends_with("crates/backup/src/checkpoint/staging/io.rs") {
        return checked_closed(
            source,
            &[
                ".union(OFlags::NOFOLLOW)",
                "!access.directory(st.st_uid,st.st_gid,st.st_mode)",
                "acl::require_no_acl(&fd,true)?;",
                "self.access.require_writer()?;",
                "component(name)?;",
                "child.pin.0!=self.pin.0",
                "a.starts_with(b)||b.starts_with(a)||ancestor(a,right.pin)?||ancestor(b,left.pin)?",
                "ifname.is_empty()||name.len()>255||matches!(name,\".\"|\"..\")",
            ],
            &[
                ("rustix::{fd::OwnedFd,fs::{self,FileType,Mode,OFlags}}", 1),
                ("fs::open(\"/\",FLAGS,Mode::empty())", 2),
                ("fs::openat(&fd,name,FLAGS,Mode::empty())", 2),
                ("fs::fstat(&fd)", 2),
                ("fs::openat(&self.fd,name,FLAGS,Mode::empty())", 1),
                ("fs::mkdirat(&self.fd,name,Mode::RWXU)", 1),
                ("fs::Dir::read_from(&self.fd)", 1),
                ("fs::statat(&self.fd,name,fs::AtFlags::SYMLINK_NOFOLLOW)", 2),
                ("fs::fsync(&self.fd)", 1),
                ("fs::fchmod(&self.fd,Mode::RUSR|Mode::XUSR)", 1),
                ("fs::fstat(&self.fd)", 1),
                ("fs::fstatvfs(&self.fd)", 1),
                (
                    "fs::renameat_with(&self.fd,pending,&self.fd,final_name,fs::RenameFlags::NOREPLACE)",
                    1,
                ),
            ],
        );
    }
    if path.ends_with("crates/backup/src/checkpoint/staging/io/files.rs") {
        return checked_closed(
            source,
            &[
                "component(name)?;",
                "parent.capacity(size)?;",
                "pin.nlink()!=1||pin.dev()!=device",
                "!access.file(pin.uid(),pin.gid(),pin.mode())",
                "super::acl::require_no_acl(&file,false)?;",
                "self.access.require_writer()?;",
                "!same(&self.pin,&after)||!same(&after,&reopened.pin)",
            ],
            &[
                ("fs::{File,Metadata}", 1),
                ("os::unix::fs::MetadataExt", 1),
                (
                    "fs::openat(&parent.fd,name,OFlags::RDONLY|OFlags::NOFOLLOW|OFlags::CLOEXEC|OFlags::NONBLOCK,Mode::empty())",
                    1,
                ),
                (
                    "fs::openat(&parent.fd,name,OFlags::WRONLY|OFlags::CREATE|OFlags::EXCL|OFlags::NOFOLLOW|OFlags::CLOEXEC,Mode::RUSR|Mode::WUSR)",
                    1,
                ),
                ("File::from(fd)", 2),
                ("self.file.write_all(bytes)", 1),
                ("self.file.sync_all()", 4),
                (
                    "fs::fchown(&self.file,None,Some(rustix::process::Gid::from_raw(reader_gid)))",
                    1,
                ),
                ("fs::fchmod(&self.file,Mode::RUSR|Mode::RGRP)", 1),
                ("fs::fchmod(&self.file,Mode::RUSR)", 1),
            ],
        );
    }
    if path.ends_with("crates/backup/src/checkpoint/staging/creation.rs") {
        return checked_closed(
            source,
            &[
                "Lifecycle::begin(parent,checkpoint_id,reader)?;",
                "Err(StagingError::CleanupFailed)",
                "self.parent.check_path(&self.parent_path)",
                "ifself.lifecycle.published",
            ],
            &[
                ("parent.create_dir(export)", 1),
                (".create_dir(name)", 1),
                ("parent.create_dir(pending)", 1),
                (".create_dir(\"data\")", 1),
            ],
        );
    }
    if path.ends_with("crates/backup/src/checkpoint/staging/lifecycle.rs") {
        return checked_closed(
            source,
            &[
                "parent.acquire_lease()?;",
                "name.starts_with(\"incomplete-\")||name.starts_with(\"export-\")||name.starts_with(\"pending-\")",
                "returnErr(StagingError::CleanupRequired)",
                "self.lease.check(parent)?;",
                "self.marker.check_child(parent,name)?;",
            ],
            &[("parent.create_dir(&name)", 1)],
        );
    }
    // 取回副本是把封存内容放到新目录的唯一出口：只许建这两处目录，写的每个文件都对着已核验视图。
    if path.ends_with("crates/backup/src/checkpoint/staging/seal/restored.rs") {
        return checked_closed(
            source,
            &[
                "hash!=reference.seal_sha256",
                "tree::verify(&directory.child(\"data\")?,deadline)?!=inclusion.view",
                "iftarget.exists(name)?{returnOk(Placement::AlreadyPresent);}",
                "iftarget.exists(&partial)?{returnErr(StagingError::CleanupRequired);}",
                "ifreplica.directories!=directories||replica.files!=files",
                "target.publish(&partial,name)?;",
                "ifdepth>64",
                "if!expected.directories.contains(&path){returnErr(StagingError::Changed);}",
                "child.check_child(source,name)?;",
                "size!=expected.bytes||hex(&hash.finalize())!=expected.sha256",
            ],
            &[
                ("target.create_dir(&partial)", 1),
                ("target.create_dir(name)", 1),
            ],
        );
    }
    if path.ends_with("crates/backup/src/checkpoint/staging/tree.rs") {
        return checked_closed(
            source,
            &[
                "ifdepth>64",
                "child.check_child(source,name)?;",
                "catalog.inspect(&path,revision.ok_or(StagingError::Catalog)?.finish(),self.now)?",
            ],
            &[
                ("t.create_dir(\"summaries\")", 1),
                ("dir.create_dir(name)", 1),
            ],
        );
    }
    source.to_owned()
}
