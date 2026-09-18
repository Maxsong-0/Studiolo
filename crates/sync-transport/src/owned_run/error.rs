//! 固定错误分类：不带原 JSON、凭证、路径或原始 stderr。
//!
//! 「未知」不是「失败/成功二选一」：`ExitUnconfirmed` 保留活动槽与证据，
//! 由 `recover`/`stop_recovered` 继续查同一容器。

use std::fmt;

/// 传输 owner 的全部固定失败分类。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TransportError {
    /// 缺 cgroup v2/委派/控制器/kill 能力、namespace、专用身份或磁盘 quota；不启动第三方程序。
    UnsupportedContainment,
    /// 固定 `state.json` 或 `transport.lock` 不存在；须管理员显式初始化。
    MissingStore,
    /// 已存在固定文件，拒绝重复初始化。
    AlreadyExists,
    /// 半 JSON、未知字段、超预算或自相矛盾的持久记录。
    CorruptStore,
    /// `schema_version` 不是已知版本；不得自动迁移或回写降级。
    UnknownSchema,
    /// 上次发布留下的临时文件仍在；它不是授权状态，必须显式丢弃后才能继续。
    TemporaryResidue,
    /// 私有根、权限、对象身份或真实 UID 不满足存储约束。
    UnsafeStorage,
    /// 另一个进程持有排他租约。
    Busy,
    /// 底层存储暂时不可用；本次发布没有生效。
    Unavailable,
    /// 请求超过固定预算。
    Budget,
    /// 持久绑定与当前批准绑定不一致。
    BindingMismatch,
    /// 私有配置在使用前后发生变化。
    ConfigurationChanged,
    /// 根的真实对象身份与持久 pins 不一致。
    RootsChanged,
    /// 固定运行时摘要或身份与持久记录不一致。
    RuntimeChanged,
    /// 持久 generation 与预期不一致；并发写入或票据过期。
    GenerationConflict,
    /// `prepare_once` 的期望序号与实际下一个序号不一致。
    SequenceMismatch,
    /// 票据来自另一个 owner 实例、另一次租约或另一个运行。
    TicketForeign,
    /// 票据已被更晚的 prepare 或根/租约变化作废。
    TicketStale,
    /// 该票据已经放行过一次；重复请求不得产生第二次执行。
    AlreadyStarted,
    /// 活动槽里已有未闭合的运行。
    RunActive,
    /// 没有对应的活动运行。
    NotFound,
    /// 阶段不允许本次转换。
    PhaseConflict,
    /// 还有待补审计事件；ACK 前不启新运行、不发静止能力。
    AuditPending,
    /// 提交的审计确认与当前待补事件不一致。
    AuditMismatch,
    /// 审计回执本身不合法（偏移为零或哈希全零）。
    AuditRejected,
    /// 无法证明容器身份或创建屏障已关闭；保留原槽，不取 token，不启新运行。
    ExitUnconfirmed,
    /// 尚未取得静止证明。
    QuiescenceUnconfirmed,
    /// 计数溢出；序号与代次一律 `checked_add`。
    CounterOverflow,
}

impl fmt::Display for TransportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::UnsupportedContainment => "受控 Linux 隔离前置未满足，不启动第三方程序",
            Self::MissingStore => "传输私有存储不存在；需要显式初始化",
            Self::AlreadyExists => "传输私有存储已存在",
            Self::CorruptStore => "传输持久记录不完整或自相矛盾",
            Self::UnknownSchema => "传输持久记录的 schema 版本未知",
            Self::TemporaryResidue => "存在未发布的临时状态文件，必须显式丢弃",
            Self::UnsafeStorage => "传输私有存储的身份或权限不满足约束",
            Self::Busy => "传输排他租约被占用",
            Self::Unavailable => "传输私有存储暂时不可用",
            Self::Budget => "请求超过固定预算",
            Self::BindingMismatch => "持久绑定与当前批准不一致",
            Self::ConfigurationChanged => "私有配置在使用前后发生变化",
            Self::RootsChanged => "根的真实对象身份与持久 pins 不一致",
            Self::RuntimeChanged => "固定运行时与持久记录不一致",
            Self::GenerationConflict => "持久代次与预期不一致",
            Self::SequenceMismatch => "期望运行序号与实际不一致",
            Self::TicketForeign => "票据不属于本 owner 实例或本次运行",
            Self::TicketStale => "票据已被更晚的准备或根变化作废",
            Self::AlreadyStarted => "该票据已放行过一次",
            Self::RunActive => "活动槽里仍有未闭合的运行",
            Self::NotFound => "没有对应的运行记录",
            Self::PhaseConflict => "当前阶段不允许该转换",
            Self::AuditPending => "仍有待补审计事件",
            Self::AuditMismatch => "审计确认与当前待补事件不一致",
            Self::AuditRejected => "审计回执不合法",
            Self::ExitUnconfirmed => "无法证明容器身份或创建屏障已关闭",
            Self::QuiescenceUnconfirmed => "尚未取得静止证明",
            Self::CounterOverflow => "计数溢出",
        })
    }
}

impl std::error::Error for TransportError {}

/// 本 crate 的固定结果类型。
pub type Result<T> = std::result::Result<T, TransportError>;
