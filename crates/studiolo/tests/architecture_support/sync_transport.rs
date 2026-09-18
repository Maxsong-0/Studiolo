//! SyncTransportOwner 只写固定的 transport-private 单槽；运行日志正文不落这里。

use std::path::Path;

pub(super) fn remainder(crate_name: &str, path: &Path, source: &str) -> String {
    if crate_name != "studiolo-sync-transport"
        || !path.ends_with("crates/sync-transport/src/store/publish.rs")
    {
        return source.to_owned();
    }
    let mut rest = source.split_whitespace().collect::<String>();
    // 固定文件名常量必须在场，换掉固定名就拿不到豁免。
    for required in [
        "paths::TRANSPORT_LOCK",
        "paths::STATE",
        "paths::STATE_PUBLISH_TEMP",
    ] {
        if !rest.contains(required) {
            return rest;
        }
    }
    for exact in [
        "fs::{File,TryLockError},",
        "userustix::fs::{AtFlags,unlinkat};",
        "userustix::fs::{Mode,OFlags,openat};",
        "userustix::fs::{Mode,OFlags,openat,renameat};",
        "userustix::fs::{Mode,OFlags,open,openat};",
        "usestd::os::unix::fs::MetadataExt;",
        "lease.0.sync_all()",
        "temp.write_all(payload)",
        "temp.sync_all()",
        "self.root.sync_all()",
        "root.sync_all()",
    ] {
        rest = rest.replace(exact, "");
    }
    rest
}
