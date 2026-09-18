/// 策略平面错误：结构化原因，不回显宿主路径或正文。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PolicyError {
    /// 私有目录或库文件不满足权限／身份规则。
    #[error("策略私有存储不安全")]
    UnsafeStorage,
    /// 只打开已有库时库缺失。
    #[error("策略私有库不存在")]
    MissingStore,
    /// 库文件存在但为空、格式不符或损坏。
    #[error("策略私有库损坏")]
    CorruptStore,
    /// SQLite 操作失败；不回显底层路径。
    #[error("策略私有库不可用")]
    StoreUnavailable,
    /// kind 不是 rules / skills / capability_set。
    #[error("策略母本类别无效")]
    InvalidKind,
    /// subject 不满足该类别的命名规则。
    #[error("策略母本 subject 无效")]
    InvalidSubject,
    /// 正文超限或不是该类别要求的格式。
    #[error("策略正文无效：{0}")]
    InvalidBody(&'static str),
    /// desired 不存在。
    #[error("desired 不存在")]
    DesiredNotFound,
    /// desired 当前状态不可发布（已发布、已被取代或已撤回）。
    #[error("desired 当前状态 {0} 不可发布")]
    DesiredNotActivatable(String),
    /// 调用方声明的 effective 哈希与当前不一致；必须重新校验后再发布。
    #[error("effective 哈希不匹配")]
    HashMismatch {
        /// 当前 effective 哈希。
        current: String,
    },
    /// 校验未通过；逐条问题在 desired 记录的报告里。
    #[error("策略校验未通过")]
    ValidationFailed,
    /// 版本不存在。
    #[error("策略版本不存在")]
    VersionNotFound,
    /// bootstrap 只允许为从未有过版本的 subject 写首版。
    #[error("该 subject 已有版本，bootstrap 只允许首版")]
    AlreadyBootstrapped,
    /// 提案数量超过有界上限。
    #[error("待处理提案数量超限")]
    TooManyProposals,
    /// 没有处于生效中的全局吊销可解除。
    #[error("当前没有生效中的全局吊销")]
    NotRevoked,
}
