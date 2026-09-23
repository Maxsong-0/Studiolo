//! 冲突副本与保全路径的命名（P2 结论 7、ADR-025 D5／D6）；纯函数，无 IO、无时钟。
//!
//! headless 生成的副本形如 `<名> (Conflicted copy <设备名> <UTC yyyyMMddHHmm>).<ext>`；
//! Studiolo 自己生成的保全副本用固定设备名 `studiolo-canonical`，
//! 放在范围外的 `ops/bridge-conflicts/`（冲突）与 `ops/bridge-rollback/`（D5 出站前保全）。

use studiolo_core::VaultPath;

/// Studiolo 生成副本时用的设备名。
pub const CANONICAL_DEVICE: &str = "studiolo-canonical";
/// 冲突保全目录（范围外：`ops/` 是 SyncScope 硬排除）。
pub const CONFLICTS_DIR: &str = "ops/bridge-conflicts";
/// D5 出站前保全目录。
pub const ROLLBACK_DIR: &str = "ops/bridge-rollback";
/// 命名里的固定标记。
pub const MARKER: &str = "Conflicted copy";

/// 解析出的一个冲突副本名。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConflictCopy {
    /// 去掉副本标记后的原路径（含扩展名）。
    pub original: VaultPath,
    /// 生成副本的设备名（headless 的镜像设备名，或 `studiolo-canonical`）。
    pub device: String,
    /// `yyyyMMddHHmm` 的 UTC 分钟戳。
    pub stamp: String,
}

/// 生成 `<目录>/<原路径去扩展名> (Conflicted copy <设备> <戳>).<ext>`；
/// `directory` 只能是 `CONFLICTS_DIR` 或 `ROLLBACK_DIR`。
///
/// # Errors
/// 设备名含斜杠／控制字符、戳不是 12 位数字、目录不是两个固定值之一时返回 `None`。
#[must_use]
pub fn copy_path(
    directory: &str,
    original: &VaultPath,
    device: &str,
    stamp: &str,
) -> Option<VaultPath> {
    // B 轮 W4 实现（含 1024 字节路径预算与 NFC 复核）。
    let _ = (directory, original, device, stamp, MARKER);
    None
}

/// 识别 headless 或 Studiolo 生成的冲突副本；不是副本命名返回 `None`。
///
/// 只在最后一个路径分量上匹配 `<名> (Conflicted copy <设备> <12 位数字>)<.ext>`，
/// 设备名可含空格，但不含 `(`、`)`、`/`。
#[must_use]
pub fn parse_copy(path: &VaultPath) -> Option<ConflictCopy> {
    // B 轮 W4 实现。
    let _ = path;
    None
}

/// 把 Unix 秒格式化成 `yyyyMMddHHmm`（UTC）；负数或越界返回 `None`。
#[must_use]
pub fn utc_minute_stamp(unix_seconds: i64) -> Option<String> {
    // B 轮 W4 实现（不引入新依赖：用 jiff，本 crate 已依赖）。
    let _ = unix_seconds;
    None
}
