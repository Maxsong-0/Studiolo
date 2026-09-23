//! 本 crate 的基础错误类型：标识符解析、Vault 路径语法与数据标签组合校验。

/// 文本形式的标识符格式不合法时返回的错误。
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("invalid {kind}: {value}")]
pub struct IdParseError {
    // 两个字段放宽到 crate 内可见：`ids` 模块解析失败时直接构造本错误。
    pub(crate) kind: &'static str,
    pub(crate) value: String,
}

/// 相对 Vault 路径在任何文件系统访问之前就被拒绝的原因。
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PathSyntaxError {
    /// 路径为空。
    #[error("path is empty")]
    Empty,
    /// 路径是绝对路径，或含空分量、`.` 或 `..` 分量。
    #[error("path is not a clean relative path")]
    NotRelative,
    /// 路径含反斜杠、控制字符（含 NUL）、组合附加符号、Windows 盘符前缀（如 `C:/`），
    /// 或百分号编码的分隔符／点／NUL（`%2f`、`%5c`、`%2e`、`%00`），或任何非 Unicode NFC 的拼写。
    #[error("path contains a forbidden encoding or character")]
    ForbiddenEncoding,
}

/// 源标签组合自相矛盾时返回的错误。
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DataLabelError {
    /// 被排除在备份之外的内容不得同步到桌面端。
    #[error("backup_scope=none requires desktop_sync=deny")]
    DesktopOutlivesBackup,
    /// 受限内容不得批准发给云端模型提供方。
    #[error("restricted content cannot allow a cloud model provider")]
    RestrictedProviderEgress,
    /// 提供方白名单至少要列出一个提供方。
    #[error("model provider allow-list is empty")]
    EmptyProviderList,
}
