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
                // BUG-043：凭据目录认两种形态（属 worker 的 0500，或属 root、ACL 恰好只多放行 worker），由
                // `acl.rs` 逐项判；上级三层照旧要属 root、别人不可写、一条 ACL 都没有。
                "if!exclusive_access(&fd,Kind::Directory,stat.st_uid,stat.st_mode,uid){returnErr(CredentialFault::DirectoryAccess);}readonly_memory(&fd)?;}elseif!no_acl(&fd)||stat.st_uid!=0||stat.st_mode&0o7022!=0{returnErr(CredentialFault::Parents);}",
                "if!exclusive_access(&self.fd,Kind::Directory,original.st_uid,original.st_mode,self.uid){returnErr(CredentialFault::DirectoryAccess);}",
                // 访问 ACL 读不出来、比认得的形态长都算不是；默认 ACL 一律不许有；权限位连特殊位一起比。
                "letaccess=matchread{Ok(length)iflength<=raw.len()=>Some(&raw[..length]),Err(rustix::io::Errno::NODATA|rustix::io::Errno::OPNOTSUPP)=>None,_=>returnfalse};",
                "absent(fd,\"system.posix_acl_default\")&&acl::exclusive(kind,owner,mode&0o7777,access,uid)",
                // 存储：只读，而且是 ramfs，或者挂载表里核到带 noswap 的 tmpfs；别的文件系统不认。
                "!mounted.f_flag.contains(fs::StatVfsMountFlags::RDONLY)",
                "matchstat.f_typeasu64{0x8584_58f6=>true,0x0102_1994=>mount::noswap(fd),_=>false};ifmemory{Ok(())}else{Err(CredentialFault::Storage)}",
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
                (
                    "fs::fgetxattr(fd,\"system.posix_acl_access\",&mutraw[..])",
                    1,
                ),
                ("fs::fstatfs(fd)", 1),
                ("fs::fstatvfs(fd)", 1),
                ("fs::StatVfsMountFlags::RDONLY", 1),
            ],
        );
    }
    if path.ends_with("crates/backup-worker/src/credentials/mount.rs") {
        // BUG-043：tmpfs 会不会换出只有挂载表说得清。挂载号向内核要，挂载表只读本进程自己的、必须真的来自
        // procfs、最多 1 MiB；那一行要唯一、是 tmpfs、超级块选项里同时有 ro 和 noswap。
        return checked_closed(
            source,
            &[
                "constPROC:u64=0x9fa0;",
                "constTABLE_LIMIT:u64=1024*1024;",
                "ifseen.stx_mask&StatxFlags::MNT_ID.bits()==0{returnfalse;}",
                "table().is_some_and(|table|tmpfs_readonly_noswap(&table,seen.stx_mnt_id))",
                ".f_typeasu64!=PROC{returnNone;}",
                "(&file).take(TABLE_LIMIT+1).read_to_string(&muttext).ok()?;(text.len()asu64<=TABLE_LIMIT).then_some(text)",
                "let(Some(tail),None)=(matching.next(),matching.next())else{returnfalse;};",
                "let(Some(\"tmpfs\"),Some(_source),Some(options),None)=(fields.next(),fields.next(),fields.next(),fields.next())else{returnfalse;};",
                "has(\"ro\")&&has(\"noswap\")",
            ],
            &[
                ("fs::{self,AtFlags,StatxFlags}", 1),
                ("std::{fs::File,io::Read}", 1),
                (
                    "fs::statx(fd,\"\",AtFlags::EMPTY_PATH,StatxFlags::MNT_ID)",
                    1,
                ),
                ("File::open(\"/proc/self/mountinfo\")", 1),
                ("fs::fstatfs(&file)", 1),
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
                // BUG-043：文件也认两种形态（`acl.rs`），读内容之前、每次复核时都要核；单链接、普通文件照旧。
                "readonly_memory(&fd)?;Ok(File::from(fd))",
                "ifpin.is_file()&&pin.nlink()==1&&exclusive_access(file,Kind::File,pin.uid(),pin.mode(),uid){Ok(())}else{Err(CredentialFault::FileAccess)}",
                "letpin=file.metadata().map_err(|_|CredentialFault::Missing)?;private(file,&pin,uid)?;",
                "readonly_memory(&self.file)?;letafter=self.file.metadata().map_err(|_|CredentialFault::Changed)?;private(&self.file,&after,root.uid())?;",
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
                "read_checked(open_root_file(file)?,0,deadline)?",
                // T-216：仓库 CA 和配置同一目录链；交给 restic 的绝对路径钉成同一个文件。
                "pub(super)constCA_PATH:&str=\"/etc/studiolo/backup-worker-ca.pem\";",
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
                (
                    "fs::openat(&directory,\"backup-worker-ca.pem\",OFlags::RDONLY|OFlags::NOFOLLOW|OFlags::CLOEXEC|OFlags::NONBLOCK,Mode::empty())",
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
                "command.args(argv).env_clear().env(\"TMPDIR\",temporary).current_dir(\"/\").stdin(ifstreams.input.is_some(){Stdio::piped()}else{Stdio::null()}).stdout(Stdio::piped()).stderr(Stdio::piped()).process_group(0);",
                "letcleanup=child.finish();",
                "letstatus=cleanup?;",
                "child.kill_group()?;",
                "ifInstant::now()>=deadline",
                "letresult=capture(&mutchild,deadline,check_source_time,streams);",
                // 让路只在子进程还没退出时问；退出了先结束整组，再等两根管道读空。
                "if!child.exited()?{interrupt()?;}else{child.kill_group()?;",
                "Capture::new(child.stderr()?,Discarded::new(streams.stderr_limit))?",
                "constSTDERR_LIMIT:u64=64*1024;",
                "constCHECK_STDERR_LIMIT:u64=4*1024*1024;",
                "bytes.len()>studiolo_backup::worker::final_manifest::FINAL_MANIFEST_MAX_BYTES",
                "if!input.finished(){returnErr(ProcessError::Io);}",
                "credentials.attach(&mutcommand,deadline)?;",
                "credentials.check(deadline)?;",
                // BUG-053：整份读回只有子进程退出 0、流也核完才算通过；流完整而退出码不是 0 也拒绝。
                "if!success{returnErr(ProcessError::Protocol);}stdout.finish()",
            ],
            &[
                ("Command::new(&executable)", 1),
                (
                    "command.args(argv).env_clear().env(\"TMPDIR\",temporary).current_dir(\"/\").stdin(ifstreams.input.is_some(){Stdio::piped()}else{Stdio::null()}).stdout(Stdio::piped()).stderr(Stdio::piped()).process_group(0)",
                    1,
                ),
                // 只有上面那一处问让路；别处再加一次（比如挪回每轮开头）就判红。
                ("interrupt()", 1),
            ],
        );
    }
    if path.ends_with("crates/backup-worker/src/process/input.rs") {
        return checked_closed(
            source,
            &[
                "self.remaining.len().min(16*1024)",
                "self.remaining=&self.remaining[count..]",
                "self.pipe=None",
            ],
            &[
                ("fs::{OFlags,fcntl_getfl,fcntl_setfl}", 1),
                ("fcntl_getfl(pipe)", 1),
                ("fcntl_setfl(pipe,flags|OFlags::NONBLOCK)", 1),
            ],
        );
    }
    if path.ends_with("crates/backup-worker/src/process/pipe.rs") {
        return checked_closed(
            source,
            &["ifcountasu64>self.sink.limit()-self.seen"],
            &[
                ("fs::{OFlags,fcntl_getfl,fcntl_setfl}", 1),
                ("fcntl_getfl(&pipe)", 1),
                ("fcntl_setfl(&pipe,flags|OFlags::NONBLOCK)", 1),
            ],
        );
    }
    source.to_owned()
}
