//! Vault 边界对外暴露的失败类型，以及进程内状态锁中毒时的统一映射。

use studiolo_core::{Revision, VaultPath};

/// Vault 边界对外暴露的失败类型。
#[derive(Debug, thiserror::Error)]
pub enum BrokerError {
    /// 同一 canonical 根已有存活写者。
    #[error("canonical writer is busy")]
    WriterBusy,
    /// canonical、data 或不可变控制对象与已登记配对不符。
    #[error("writer storage binding mismatch")]
    WriterBindingMismatch,
    /// 写者持久记录缺失、格式不合法或不满足协议不变量。
    #[error("corrupt writer state")]
    CorruptWriterState,
    /// 未决安装或旧未知操作阻止新的 canonical 安装。
    #[error("canonical mutation is unresolved")]
    MutationUnresolved,
    /// 原请求已明确取消，重放不会再次尝试安装。
    #[error("canonical mutation was cancelled")]
    MutationCancelled,
    /// 未知安装已封存证据并终止重放，不代表原请求曾成功或失败。
    #[error("canonical mutation remains indeterminate: {evidence_id}")]
    MutationIndeterminate {
        /// 私有协调证据 ID，不包含正文或主机路径。
        evidence_id: String,
    },
    /// 写者协议触及的输入或持久记录超过固定预算。
    #[error("writer storage or input limit exceeded")]
    WriterLimitExceeded,
    /// 捕获所属实例或已确认提交水位变化，候选视图不能发布。
    #[error("canonical capture changed")]
    CaptureChanged,
    /// 扫描出现安全诊断，不能把部分结果当作完整捕获。
    #[error("canonical capture is incomplete")]
    IncompleteCapture,
    /// 受保护的控制路径（或隐藏分量）被送入普通变更、历史、编号等业务接口。
    #[error("protected Vault path: {0}")]
    ProtectedPath(VaultPath),
    /// 历史游标语法错误或不属于本路径。
    #[error("invalid history cursor")]
    InvalidHistoryCursor,
    /// 历史目录已变化，调用方须重新从第一页开始。
    #[error("history changed")]
    HistoryChanged,
    /// 请求的对象不存在。
    #[error("Vault object not found: {0}")]
    NotFound(VaultPath),
    /// 比较并交换（CAS）前置条件不匹配。
    #[error("revision conflict at {path}: expected {expected}, found {actual:?}")]
    RevisionConflict {
        /// 发生冲突的逻辑路径。
        path: VaultPath,
        /// 要求的 revision；要求目标不存在（创建）时为 `absence`。
        expected: String,
        /// Broker 实际观察到的 revision。
        actual: Option<Revision>,
    },
    /// 路径分量不是安全节点：符号链接、硬链接、特殊文件、跨设备、隐藏或非 UTF-8 名称等。
    #[error("unsafe filesystem node at {path}: {reason}")]
    UnsafeNode {
        /// 被拒绝的逻辑路径。
        path: VaultPath,
        /// 不含敏感信息的固定原因。
        reason: &'static str,
    },
    /// 该拼写在常见文件系统上会与已有名称互为别名（如仅大小写或 Unicode 规范化形式不同）。
    #[error("path collision between {requested} and existing {existing}")]
    PathCollision {
        /// 请求的逻辑路径。
        requested: VaultPath,
        /// 已存在的逻辑拼写。
        existing: String,
    },
    /// 幂等键（或同类重放绑定）被用于不同的输入。
    #[error("idempotency key was reused with different input")]
    IdempotencyConflict,
    /// 请求不合法：名称沿用自产物提交，现也用于快照、维护、文件操作、编号等输入校验。
    #[error("invalid artifact commit: {0}")]
    InvalidArtifact(String),
    /// 已有产物或 manifest 与本次提交不一致（如同一提交重试时遇到内容不同的孤儿产物）。
    #[error("artifact already exists with different content: {0}")]
    ArtifactConflict(VaultPath),
    /// 持久化的 Vault 身份记录格式不合法。
    #[error("corrupt Vault identity")]
    CorruptIdentity,
    /// 已批准的 canonical 根不再解析到当初固定的目录。
    #[error("canonical Vault root changed or is unavailable")]
    RootChanged,
    /// 有界读取超出调用方给定的字节预算。
    #[error("read limit of {max_bytes} bytes exceeded at {path}")]
    ReadLimitExceeded {
        /// 字节被拒收的逻辑路径。
        path: VaultPath,
        /// 允许的最大内容字节数，读取期间的增长也计入。
        max_bytes: u64,
    },
    /// 已存快照的元数据或内容未通过完整性校验。
    #[error("corrupt snapshot record")]
    CorruptSnapshot,
    /// 读取、扫描或快照捕获期间源对象发生了变化（前后复核不一致）。
    #[error("source changed while snapshotting: {0}")]
    UnstableRead(VaultPath),
    /// 操作系统调用失败；错误中不暴露宿主路径。
    #[error("Vault I/O failed during {operation}: {source}")]
    Io {
        /// 稳定的操作标签。
        operation: &'static str,
        /// 底层操作系统错误。
        #[source]
        source: std::io::Error,
    },
    /// 持久记录的 JSON 解析失败（序列化失败也归入此类）。
    #[error("invalid durable record during {operation}: {source}")]
    Json {
        /// 稳定的操作标签。
        operation: &'static str,
        /// JSON 错误。
        #[source]
        source: serde_json::Error,
    },
}

impl BrokerError {
    pub(crate) fn io(operation: &'static str, source: std::io::Error) -> Self {
        Self::Io { operation, source }
    }

    pub(crate) fn json(operation: &'static str, source: serde_json::Error) -> Self {
        Self::Json { operation, source }
    }
}

pub(crate) fn memory_state_error() -> BrokerError {
    BrokerError::io(
        "access memory adapter state",
        std::io::Error::other("memory adapter mutex poisoned"),
    )
}
