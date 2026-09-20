use studiolo_core::PathSyntaxError;
use studiolo_vault_broker::BrokerError;

/// 供给面错误：结构化原因，不回显宿主路径。
#[derive(Debug, thiserror::Error)]
pub enum SupplyError {
    /// 白名单配置本身不合法（空、隐藏分量、重叠或重复前缀）。
    #[error("白名单配置无效：{0}")]
    InvalidAllowlist(&'static str),
    /// 请求的逻辑路径语法不合法。
    #[error("路径语法无效：{0}")]
    PathSyntax(#[source] PathSyntaxError),
    /// 请求的路径不在白名单内；未列入即拒绝，不存在默认只读。
    #[error("路径不在供给白名单内")]
    OutsideAllowlist,
    /// 白名单内的路径不存在。
    #[error("路径不存在")]
    NotFound,
    /// 只读根或扫描失败关闭（链接、越界、预算、不稳定读取等）。
    #[error("只读供给失败：{0}")]
    Broker(#[source] BrokerError),
    /// Markdown 内容不是 UTF-8。
    #[error("笔记不是 UTF-8")]
    NotUtf8,
    /// 私有目录或库文件不满足权限/身份规则。
    #[error("agent 私有存储不安全")]
    UnsafeStorage,
    /// 只打开已有库时库缺失。
    #[error("agent 私有库不存在")]
    MissingStore,
    /// 库文件存在但为空或损坏。
    #[error("agent 私有库损坏")]
    CorruptStore,
    /// SQLite 操作失败；不回显底层路径。
    #[error("agent 私有库不可用")]
    StoreUnavailable,
    /// 子实例需要显式升级后的身份库。
    #[error("子实例功能需要显式升级身份库")]
    IdentityUpgradeRequired,
    /// 父实例不存在、类型不符或树深度越界。
    #[error("子实例关系无效")]
    InvalidParent,
    /// 类型名不合法。
    #[error("类型名只允许 1..=32 位小写字母、数字与连字符")]
    InvalidTypeName,
}
