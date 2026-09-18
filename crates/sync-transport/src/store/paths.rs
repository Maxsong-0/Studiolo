//! 纯路径：固定三个名字，没有任何可由调用方拼接的文件名入口。

use std::path::Path;

/// 排他租约文件名。
pub const TRANSPORT_LOCK: &str = "transport.lock";
/// 权威状态文件名。
pub const STATE: &str = "state.json";
/// 唯一已登记的发布临时文件名；它永远不是授权状态。
pub const STATE_PUBLISH_TEMP: &str = "state.json.publish";

/// 租约文件字节上限。
pub const LOCK_MAX_BYTES: u64 = 4096;
/// 状态文件字节上限。
pub const STATE_MAX_BYTES: u64 = 64 * 1024;

/// 本存储允许出现的全部固定名字。
#[must_use]
pub fn fixed_names() -> [&'static str; 3] {
    [TRANSPORT_LOCK, STATE, STATE_PUBLISH_TEMP]
}

/// 指定名字的字节上限；未知名字没有预算，不允许出现。
#[must_use]
pub fn max_bytes(name: &str) -> Option<u64> {
    match name {
        TRANSPORT_LOCK => Some(LOCK_MAX_BYTES),
        STATE | STATE_PUBLISH_TEMP => Some(STATE_MAX_BYTES),
        _ => None,
    }
}

/// 传输私有根必须是绝对、无 `.`/`..`、无控制字符的路径。
#[must_use]
pub fn private_root_acceptable(path: &Path) -> bool {
    crate::owned_run::absolute_normal(path)
}
