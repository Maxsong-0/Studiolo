//! Unix入口只获固定父FD的socket权限与清理；不能借Adapter名放行通用写入。

use std::path::Path;

pub(super) fn remainder(crate_name: &str, path: &Path, source: &str) -> String {
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
