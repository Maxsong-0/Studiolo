//! BackupWorker 只写自己登记的固定备份状态与清单；协调器不持有任何 owner 的数据库句柄。

use std::path::Path;

pub(super) fn remainder(crate_name: &str, path: &Path, source: &str) -> String {
    if crate_name != "studiolo-backup" || !path.ends_with("crates/backup/src/store/storage.rs") {
        return source.to_owned();
    }
    let rest = source.split_whitespace().collect::<String>();
    // 固定文件名必须在场，换掉文件名就拿不到豁免。
    for required in [
        "\"backup-state.json\"",
        "\"backup-state.json.tmp\"",
        "\"manifest.json\"",
        "\"manifest.json.tmp\"",
    ] {
        if !rest.contains(required) {
            return rest;
        }
    }
    rest.replace(
        "userustix::fs::{AtFlags,FileType,Mode,OFlags,fstat,fsync,open,openat,renameat,unlinkat};",
        "",
    )
}
