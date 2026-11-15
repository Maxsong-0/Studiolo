//! 本 crate 的持久物只有两份：死人开关状态和出站门铃状态（ADR-033），都只经
//! `store/storage.rs` 按固定文件名读写；健康聚合、告警派发与门铃投递本身都不落盘。

use std::path::Path;

pub(super) fn remainder(crate_name: &str, path: &Path, source: &str) -> String {
    if crate_name != "studiolo-observability"
        || !path.ends_with("crates/observability/src/store/storage.rs")
    {
        return source.to_owned();
    }
    let rest = source.split_whitespace().collect::<String>();
    // 固定文件名必须在场，换掉文件名就拿不到豁免。
    for required in [
        "STATE_FILE_NAME:&str=\"deadman-state.json\";",
        "STAGE_FILE_NAME:&str=\"deadman-state.json.new\";",
        "DOORBELL_STATE_FILE_NAME:&str=\"doorbell-state.json\";",
        "DOORBELL_STAGE_FILE_NAME:&str=\"doorbell-state.json.new\";",
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
