//! 新副本封存能力逐调用登记；任何额外任意路径写入仍由总守卫拒绝。
use super::super::authority_tokens::checked_closed;
use std::path::Path;
pub(super) fn remainder(path: &Path, source: &str) -> String {
    if path.ends_with("crates/backup/src/checkpoint/staging/io/cleanup.rs") {
        return checked_closed(
            source,
            &[
                "self.check_child(parent,name)?;",
                "self.access.require_writer()?;",
                "parent.access.require_writer()?;",
                "letmutremaining=MAX_NODES;",
                "ifdepth>64",
                "child.check_child(self,name)?;",
            ],
            &[
                ("fs::unlinkat(&parent.fd,name,fs::AtFlags::REMOVEDIR)", 1),
                ("fs::fchmod(&self.fd,Mode::RWXU)", 1),
                ("fs::statat(&self.fd,name,fs::AtFlags::SYMLINK_NOFOLLOW)", 1),
                ("fs::unlinkat(&self.fd,name,fs::AtFlags::REMOVEDIR)", 1),
                ("fs::unlinkat(&self.fd,name,fs::AtFlags::empty())", 1),
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
                ("self.file.sync_all()", 3),
                (
                    "fs::fchown(&self.file,None,Some(rustix::process::Gid::from_raw(reader_gid)))",
                    1,
                ),
                ("fs::fchmod(&self.file,Mode::RUSR|Mode::RGRP)", 1),
                ("fs::fchmod(&self.file,Mode::RUSR)", 1),
            ],
        );
    }
    if path.ends_with("crates/backup/src/checkpoint/staging.rs") {
        return checked_closed(
            source,
            &[
                "io::disjoint(parent,&parent_dir,path,&root)?;",
                "io::disjoint(ephemeral_root,&ephemeral,path,&root)?;",
                "letkey=digest(checkpoint_id.as_str().as_bytes());",
            ],
            &[
                ("parent_dir.create_dir(&export_name)", 1),
                ("exports.create_dir(name)", 1),
                ("parent_dir.create_dir(&pending_name)", 1),
                ("pending.create_dir(\"data\")", 1),
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
