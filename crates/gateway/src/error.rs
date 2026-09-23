//! Gateway 边界与实时 attempt 权威的错误类型。

/// 加载实时 attempt 状态失败。
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AttemptGuardError {
    /// 该 task/attempt 组合未处于活动状态或不存在。
    #[error("attempt is not active")]
    UnknownAttempt,
    /// 运行时状态的权威来源当前无法应答。
    #[error("attempt guard is unavailable")]
    Unavailable,
}

/// Gateway 边界返回的错误。
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum GatewayError {
    /// 调用标识符无效：为空（或全为空白）、含控制字符，或无法据此构成审计事件 ID。
    #[error("gateway call id is invalid")]
    InvalidCallId,
    /// 该 task/attempt 组合未处于活动状态或不存在。
    #[error("attempt is not active")]
    AttemptNotActive,
    /// 无法检查运行时状态，请求按失败关闭处理。
    #[error("attempt guard is unavailable")]
    AttemptGuardUnavailable,
    /// 该 attempt 已取消，其 capability 已被撤销。
    #[error("attempt is cancelled")]
    Cancelled,
    /// 出示的 fencing token 已过时。
    #[error("capability fence does not match the live attempt")]
    FenceMismatch,
    /// 短期 capability 已过期。
    #[error("job capability is expired")]
    CapabilityExpired,
    /// 至少有一层权限拒绝了所请求的工具。
    #[error("tool is not allowed")]
    ToolNotAllowed,
    /// 权威审计持久化失败，因此调用未被放行。
    #[error("authoritative audit is unavailable")]
    AuditUnavailable,
    /// 内存工具适配器没有配置响应。
    #[error("tool adapter is unavailable")]
    ToolUnavailable,
    /// 内存模型适配器没有配置响应。
    #[error("model relay is unavailable")]
    ModelUnavailable,
    /// 同一调用标识符被用于内容不同的请求。
    #[error("call id conflicts with a prior request")]
    CallConflict,
    /// 完全相同的调用已经完成，但保留的结果已被逐出。
    #[error("call already completed and its result is no longer retained")]
    CallResultEvicted,
    /// 内存适配器状态不可用（互斥锁中毒等）。
    #[error("gateway state is unavailable")]
    StateUnavailable,
}

impl GatewayError {
    pub(crate) fn audit_reason(&self) -> &'static str {
        match self {
            Self::InvalidCallId => "invalid_call_id",
            Self::AttemptNotActive => "attempt_not_active",
            Self::AttemptGuardUnavailable => "attempt_guard_unavailable",
            Self::Cancelled => "cancelled",
            Self::FenceMismatch => "fence_mismatch",
            Self::CapabilityExpired => "capability_expired",
            Self::ToolNotAllowed => "tool_not_allowed",
            Self::AuditUnavailable => "audit_unavailable",
            Self::ToolUnavailable => "tool_unavailable",
            Self::ModelUnavailable => "model_unavailable",
            Self::CallConflict => "call_conflict",
            Self::CallResultEvicted => "call_result_evicted",
            Self::StateUnavailable => "state_unavailable",
        }
    }
}
