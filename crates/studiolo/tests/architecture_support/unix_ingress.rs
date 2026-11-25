//! Unix入口只获固定父FD的socket权限与清理；不能借Adapter名放行通用写入。

use super::authority_tokens::checked_closed;
use std::path::Path;

/// 死 socket 接管（T-231）：判断一步不少，才放行相对父目录句柄的那一处删除。
const LEFTOVER: &[&str] = &[
    "ifFileType::from_raw_mode(stat.st_mode)!=FileType::Socket||stat.st_uid!=geteuid().as_raw()||stat.st_nlink!=1||!same(&stat,&by_path){returnErr(Refused);}Ok(stat)",
    "letfound=self.candidate()?;if!probe(self.path){returnErr(Refused);}",
    "if!same(&found,&self.candidate()?){returnErr(Refused);}Ok(found)",
    "letsole=workbench.sole_instance().map_err(|_|Refused)?;letretired=self.retire_leftover(&sole,unheard)?;drop(sole);",
    "_sole:&SoleInstance<'_>,probe:implFnOnce(&Path)->bool)->Result<Stat,Refused>{letfound=self.judge(probe)?;unlinkat(self.parent,self.name,AtFlags::empty())",
    "left.st_dev==right.st_dev&&left.st_ino==right.st_ino&&left.st_ctime==right.st_ctime&&left.st_ctime_nsec==right.st_ctime_nsec",
    "letOk(address)=SocketAddrUnix::new(path)else{returnfalse;};",
    "letOk(probe)=socket_with(AddressFamily::UNIX,SocketType::STREAM,SocketFlags::NONBLOCK|SocketFlags::CLOEXEC,None)else{returnfalse;};",
    "matches!(connect(&probe,&address),Err(Errno::CONNREFUSED))",
];

fn leftover(source: &str) -> String {
    checked_closed(
        source,
        LEFTOVER,
        &[
            ("fs::{AtFlags,CWD,FileType,Stat,statat,unlinkat}", 1),
            ("statat(self.parent,self.name,AtFlags::SYMLINK_NOFOLLOW)", 1),
            ("statat(CWD,self.path,AtFlags::SYMLINK_NOFOLLOW)", 1),
            ("unlinkat(self.parent,self.name,AtFlags::empty())", 1),
            ("Errno::CONNREFUSED", 1),
            ("connect(", 1),
        ],
    )
}

pub(super) fn remainder(crate_name: &str, path: &Path, source: &str) -> String {
    if crate_name == "studiolo-local-server"
        && path.ends_with("crates/local-server/src/unix_ingress/leftover.rs")
    {
        return leftover(source);
    }
    if crate_name != "studiolo-local-server"
        || !path.ends_with("crates/local-server/src/unix_ingress/layout.rs")
    {
        return source.to_owned();
    }
    let mut remainder = source
        .split_whitespace()
        .collect::<String>()
        .replace(",)", ")");
    let open = "open(&parent.path,OFlags::RDONLY|OFlags::DIRECTORY|OFlags::NOFOLLOW|OFlags::CLOEXEC,Mode::empty())";
    let chmod = "chmodat(&cleanup.layout.descriptor,cleanup.layout.name(),Mode::RUSR|Mode::WUSR,AtFlags::empty())";
    let unlink = "unlinkat(&self.layout.descriptor,self.layout.name(),AtFlags::empty())";
    if remainder.matches("open(").count() != 1
        || !remainder.contains(open)
        || remainder.matches("chmodat(").count() != 1
        || !remainder.contains(chmod)
        || remainder.matches("unlinkat(").count() != 1
        || !remainder.contains(unlink)
        || remainder.matches("statat(").count() != 2
        || remainder
            .matches("statat(&self.descriptor,self.name(),AtFlags::SYMLINK_NOFOLLOW)")
            .count()
            != 2
    {
        return remainder;
    }
    for allowed in [
        "fs::{AtFlags,FileType,Mode,OFlags,Stat,chmodat,open,statat,unlinkat}",
        "fs::{self,File}",
        "os::unix::fs::MetadataExt",
        "fs::symlink_metadata(",
        open,
        chmod,
        unlink,
    ] {
        remainder = remainder.replace(allowed, "");
    }
    remainder
}
