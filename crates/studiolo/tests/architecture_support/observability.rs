//! 死人开关状态是本 crate 唯一的持久物；健康聚合与告警派发都不落盘。

use std::path::Path;

pub(super) fn remainder(crate_name: &str, path: &Path, source: &str) -> String {
    if crate_name != "studiolo-observability"
        || !path.ends_with("crates/observability/src/store/storage.rs")
    {
        return source.to_owned();
    }
    let rest = source.split_whitespace().collect::<String>();
    // 固定文件名必须在场，换掉文件名就拿不到豁免。
    for required in ["STATE_FILE_NAME", "STAGE_FILE_NAME"] {
        if !rest.contains(required) {
            return rest;
        }
    }
    rest.replace(
        "userustix::fs::{AtFlags,FileType,Mode,OFlags,fstat,fsync,open,openat,renameat,unlinkat};",
        "",
    )
}
