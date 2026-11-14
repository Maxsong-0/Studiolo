//! 独立 worker 的原 FD 执行、环境与进程组清理逐调用登记，不给 App 泛化执行能力。
use super::super::authority_tokens::checked_closed;
use std::path::Path;

pub(super) fn remainder(path: &Path, source: &str) -> String {
    if path.ends_with("crates/backup-worker/src/credentials/root.rs") {
        return checked_closed(
            source,
            &[
                "constROOT:&str=\"/run/credentials/studiolo-backup-worker.service\";",
                ".union(OFlags::NOFOLLOW)",
                "[None,Some(\"run\"),Some(\"credentials\"),Some(\"studiolo-backup-worker.service\")]",
                "stat.st_uid!=uid||stat.st_mode&0o7777!=0o500",
                "stat.st_uid!=0||stat.st_mode&0o7022!=0",
                "readonly_memory(&fd)?;",
                "stat.f_typeasu64!=0x8584_58f6",
                "!mount.f_flag.contains(fs::StatVfsMountFlags::RDONLY)",
                "reopened.pins!=self.pins",
                "[\"system.posix_acl_access\",\"system.posix_acl_default\"]",
            ],
            &[
                ("fs::{self,Mode,OFlags}", 1),
                ("fs::open(\"/\",FLAGS,Mode::empty())", 1),
                ("fs::openat(&fd,name,FLAGS,Mode::empty())", 1),
                ("fs::fstat(&fd)", 1),
                ("fs::fstat(&self.fd)", 1),
                ("fs::fgetxattr(fd,name,&mutbyte[..])", 1),
                ("fs::fstatfs(fd)", 1),
                ("fs::fstatvfs(fd)", 1),
                ("fs::StatVfsMountFlags::RDONLY", 1),
            ],
        );
    }
    if path.ends_with("crates/backup-worker/src/credentials/file.rs") {
        return checked_closed(
            source,
            &[
                "Self::Repository=>\"repository-password\"",
                "Self::Username=>\"rest-username\"",
                "Self::Password=>\"rest-password\"",
                "readonly_memory(&fd)?;no_acl(&fd)?;",
                "pin.nlink()!=1||pin.mode()&0o7777!=0o400",
                "count>limit+1-bytes.len()",
                "!same(&self.pin,&after)||!same(&self.pin,&reopened)",
            ],
            &[
                ("rustix::fs::{self,Mode,OFlags}", 1),
                ("fs::{File,Metadata}", 1),
                ("os::unix::fs::MetadataExt", 1),
                (
                    "fs::openat(root.fd(),slot.name(),OFlags::RDONLY|OFlags::NOFOLLOW|OFlags::CLOEXEC|OFlags::NONBLOCK,Mode::empty())",
                    1,
                ),
                ("File::from(fd)", 1),
            ],
        );
    }
    if path.ends_with("crates/backup-worker/src/source/parents.rs") {
        return checked_closed(
            source,
            &[
                ".union(OFlags::NOFOLLOW)",
                "parts.len()<2||parts.len()>64",
                "stat.st_uid!=0&&stat.st_uid!=producer",
                "stat.st_mode&0o7022!=0",
                "item.uid!=producer||item.gid!=group||item.mode&0o7777!=mode",
                "ifindex+1==parts.len(){0o550}else{0o750}",
                "[\"system.posix_acl_access\",\"system.posix_acl_default\"]",
                "Err(rustix::io::Errno::NODATA|rustix::io::Errno::OPNOTSUPP)",
            ],
            &[
                ("fs::{self,Mode,OFlags,Stat}", 1),
                ("fs::open(\"/\",FLAGS,Mode::empty())", 1),
                ("fs::openat(&fd,*name,FLAGS,Mode::empty())", 1),
                ("fs::fstat(fd)", 1),
                ("fs::fgetxattr(fd,name,&mutbyte[..])", 1),
            ],
        );
    }
    if path.ends_with("crates/backup-worker/src/config/input.rs") {
        return checked_closed(
            source,
            &[
                ".union(OFlags::NOFOLLOW)",
                "[None,Some(\"etc\"),Some(\"studiolo\")]",
                "read_checked(open_config()?,0,deadline)?",
                "stat.st_uid!=0||stat.st_mode&0o7022!=0",
                "!same(&pin,&reopened)",
                "pin.len()>16*1024",
                "(&mutfile).take(16*1024+1).read_to_end(&mutbytes)",
            ],
            &[
                ("rustix::fs::{self,Mode,OFlags}", 1),
                ("fs::{File,Metadata}", 1),
                ("os::unix::fs::MetadataExt", 1),
                ("fs::open(\"/\",FLAGS,Mode::empty())", 1),
                ("fs::openat(&directory,name,FLAGS,Mode::empty())", 1),
                ("fs::fstat(&directory)", 1),
                (
                    "fs::openat(&directory,\"backup-worker.json\",OFlags::RDONLY|OFlags::NOFOLLOW|OFlags::CLOEXEC|OFlags::NONBLOCK,Mode::empty())",
                    1,
                ),
                ("File::from(fd)", 1),
            ],
        );
    }
    if path.ends_with("crates/backup-worker/src/credentials.rs") {
        return checked_closed(
            source,
            &[
                "_residency:&Residency",
                "password:Zeroizing<String>",
                "input::Input::load(config.worker_uid(),deadline)?",
                "self.check(deadline)?;",
            ],
            &[(
                "command.env(\"RESTIC_PASSWORD_FILE\",self.password_file.as_str()).env(\"RESTIC_REST_USERNAME\",self.username.as_str()).env(\"RESTIC_REST_PASSWORD\",self.password.as_str())",
                1,
            )],
        );
    }
    if path.ends_with("crates/backup-worker/src/temporary.rs") {
        return checked_closed(
            source,
            &[
                ".union(OFlags::NOFOLLOW)",
                "constROOT:&str=\"/var/lib/studiolo-backup-worker/tmp\";",
                "ifuid==0",
                "[\"var\",\"lib\",\"studiolo-backup-worker\",\"tmp\"]",
                "stat.st_uid!=uid||stat.st_mode&0o7777!=0o700",
                "Self::open()?.pin!=self.pin",
            ],
            &[
                ("fs::{self,Mode,OFlags}", 1),
                ("fs::open(\"/\",FLAGS,Mode::empty())", 1),
                ("fs::openat(&fd,part,FLAGS,Mode::empty())", 1),
                ("fs::fstat(&fd)", 2),
                ("fs::fstat(&self.fd)", 1),
            ],
        );
    }
    if path.ends_with("crates/backup-worker/src/process/child.rs") {
        return checked_closed(
            source,
            &[
                "WaitIdOptions::EXITED|WaitIdOptions::NOHANG|WaitIdOptions::NOWAIT",
                "self.owned=false;",
                "self.exited()?;self.kill_group()?;",
                "while!self.exited()?",
                "Duration::from_secs(2)",
            ],
            &[
                ("command.spawn()", 1),
                ("child.kill()", 1),
                ("child.wait()", 2),
                (
                    "process::waitid(WaitId::Pid(self.pid),WaitIdOptions::EXITED|WaitIdOptions::NOHANG|WaitIdOptions::NOWAIT)",
                    1,
                ),
                ("process::kill_process_group(self.pid,Signal::KILL)", 1),
            ],
        );
    }
    if path.ends_with("crates/backup-worker/src/package.rs") {
        return checked_closed(
            source,
            &[
                ".union(OFlags::NOFOLLOW)",
                "constPACKAGE_PATH:&str=\"/usr/lib/studiolo-backup-worker/restic-0.19.1\";",
                "Self::from_file(File::from(fd),expected_sha256,0,deadline)",
                "ifbytes!=pin.len()||actual!=hash",
                "!same(&self.pin,&stat)",
                "stat.st_uid!=0||stat.st_mode&0o7022!=0",
                "#[cfg(test)]pub(crate)fnfixture",
            ],
            &[
                ("rustix::fs::{self,Mode,OFlags}", 1),
                ("fs::{File,Metadata}", 1),
                ("unix::fs::MetadataExt", 1),
                ("fs::open(\"/\",FLAGS,Mode::empty())", 1),
                ("fs::openat(&directory,name,FLAGS,Mode::empty())", 1),
                (
                    "fs::openat(&directory,\"restic-0.19.1\",OFlags::RDONLY|OFlags::NOFOLLOW|OFlags::CLOEXEC|OFlags::NONBLOCK,Mode::empty())",
                    1,
                ),
                ("File::from(fd)", 1),
                ("fs::fstat(fd)", 1),
                ("File::open(path)", 1),
            ],
        );
    }
    if path.ends_with("crates/backup-worker/src/process.rs") {
        return checked_closed(
            source,
            &[
                "letexecutable=package.executable()?;",
                "command.args(argv).env_clear().env(\"TMPDIR\",temporary).current_dir(\"/\").stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).process_group(0);",
                "letcleanup=child.finish();",
                "letstatus=cleanup?;",
                "child.kill_group()?;",
                "ifInstant::now()>=deadline",
                "letresult=capture(&mutchild,deadline,check_source_time);",
                "check_source_time()?;",
                "credentials.attach(&mutcommand,deadline)?;",
                "credentials.check(deadline)?;",
            ],
            &[
                ("Command::new(&executable)", 1),
                (
                    "command.args(argv).env_clear().env(\"TMPDIR\",temporary).current_dir(\"/\").stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).process_group(0)",
                    1,
                ),
            ],
        );
    }
    if path.ends_with("crates/backup-worker/src/process/pipe.rs") {
        return checked_closed(
            source,
            &["ifcount>self.limit-self.bytes.len()"],
            &[
                ("fs::{OFlags,fcntl_getfl,fcntl_setfl}", 1),
                ("fcntl_getfl(&pipe)", 1),
                ("fcntl_setfl(&pipe,flags|OFlags::NONBLOCK)", 1),
            ],
        );
    }
    source.to_owned()
}
