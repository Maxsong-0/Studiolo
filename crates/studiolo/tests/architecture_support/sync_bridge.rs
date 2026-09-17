//! BridgeState 仅可固定打开自己的数据库/租约；纯规划器没有 I/O 权限。

use std::path::Path;

pub(super) fn remainder(crate_name: &str, path: &Path, source: &str) -> String {
    if crate_name != "studiolo-sync-bridge"
        || !path.ends_with("crates/sync-bridge/src/sqlite/storage.rs")
    {
        return source.to_owned();
    }
    let mut rest = source
        .split_whitespace()
        .collect::<String>()
        .replace(",)", ")");
    if !rest.contains("database:path.join(\"bridge.sqlite3\")") {
        return rest;
    }
    for exact in [
        "rustix::fs::{Mode,OFlags,openat}",
        "rustix::fs::{Mode,OFlags,open,openat}",
        "fs::File,",
        "fs::{File,TryLockError}",
        "std::os::unix::fs::MetadataExt",
        "openat(&root,\"bridge.lock\",flags,Mode::from_raw_mode(0o600))",
        "openat(&root,\"bridge.sqlite3\",flags,Mode::from_raw_mode(0o600))",
        "open(\"/\",flags,Mode::empty())",
        "openat(&fd,n,flags,Mode::empty())",
        "openat(root,name,OFlags::RDONLY|OFlags::NOFOLLOW|OFlags::NONBLOCK|OFlags::CLOEXEC,Mode::empty())",
        "storage._lease.0.sync_all()",
        "storage.database_pin.sync_all()",
        "storage.root.sync_all()",
    ] {
        rest = rest.replace(exact, "");
    }
    let calls = rest.replace("fnopen(", "");
    if calls.contains("openat(") || calls.contains("open(") {
        rest.push_str("fs::unexpected_open");
    }
    rest
}
