//! SyncTransportOwner 只写固定的 transport-private 单槽；运行日志正文不落这里。
//! `linux/client.rs`（T-132 P1，ADR-025 D3）是唯一的 launcher IPC 文件：只连固定 socket、
//! 只收发有界帧；不开文件、不起子进程。

use std::path::Path;

pub(super) fn remainder(crate_name: &str, path: &Path, source: &str) -> String {
    if crate_name != "studiolo-sync-transport" {
        return source.to_owned();
    }
    if path.ends_with("crates/sync-transport/src/linux/client.rs") {
        // 固定端点常量与 SO_PEERCRED 之外的连接形态一概不放行；W1 按精确调用清单收口。
        return super::authority_tokens::checked_closed(
            source,
            &["protocol::LAUNCHER_SOCKET", "FRAME_MAX_BYTES"],
            &[
                ("UnixStream::connect(socket)", 1),
                (".write_all(&frame)", 1),
                ("std::os::unix::net::UnixStream", 1),
            ],
        );
    }
    if !path.ends_with("crates/sync-transport/src/store/publish.rs") {
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
