//! 策略端点的错误响应：固定 code 与中文说明，会话、策略库与账本错误逐类映射到状态码。

use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use studiolo_audit_ledger::AuditError;
use studiolo_auth::SessionError;
use studiolo_policy::PolicyError;

pub(super) struct Problem {
    status: StatusCode,
    body: serde_json::Value,
}

impl Problem {
    pub(super) fn new(status: StatusCode, code: &'static str, error: impl Into<String>) -> Self {
        Self {
            status,
            body: serde_json::json!({"code": code, "error": error.into()}),
        }
    }
    pub(super) fn absent() -> Self {
        Self::new(
            StatusCode::NOT_FOUND,
            "policy_plane_absent",
            "本入口未装配策略平面。",
        )
    }
    pub(super) fn busy() -> Self {
        Self::new(
            StatusCode::TOO_MANY_REQUESTS,
            "busy",
            "策略操作已达并发限制，请稍后重试。",
        )
    }
    pub(super) fn unknown() -> Self {
        Self::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "outcome_unknown",
            "结果尚不能确认，请核对当前 effective 后再决定下一步。",
        )
    }
    pub(super) fn session(error: SessionError) -> Self {
        match error {
            SessionError::Rejected => {
                Self::new(StatusCode::UNAUTHORIZED, "session", "需要有效的应用会话。")
            }
            SessionError::RateLimited => Self::new(
                StatusCode::TOO_MANY_REQUESTS,
                "rate_limited",
                "认证请求已达限制。",
            ),
            _ => Self::unknown(),
        }
    }
    pub(super) fn policy(error: PolicyError) -> Self {
        match error {
            PolicyError::HashMismatch { current } => Self {
                status: StatusCode::CONFLICT,
                body: serde_json::json!({
                    "code": "hash_mismatch",
                    "error": "effective 已变化，请重新校验后再发布。",
                    "current_effective_hash": current,
                }),
            },
            PolicyError::ValidationFailed => Self::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "validation_failed",
                "校验未通过；逐条问题见该 desired 的报告。",
            ),
            PolicyError::DesiredNotFound | PolicyError::VersionNotFound => {
                Self::new(StatusCode::NOT_FOUND, "not_found", error.to_string())
            }
            PolicyError::DesiredNotActivatable(_)
            | PolicyError::NotRevoked
            | PolicyError::AlreadyBootstrapped => {
                Self::new(StatusCode::CONFLICT, "state", error.to_string())
            }
            PolicyError::InvalidKind
            | PolicyError::InvalidSubject
            | PolicyError::InvalidBody(_)
            | PolicyError::TooManyProposals => {
                Self::new(StatusCode::BAD_REQUEST, "invalid", error.to_string())
            }
            PolicyError::UnsafeStorage
            | PolicyError::MissingStore
            | PolicyError::CorruptStore
            | PolicyError::StoreUnavailable => Self::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "store_unavailable",
                "策略私有库不可用。",
            ),
        }
    }
    pub(super) fn audit(_: AuditError) -> Self {
        Self::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "audit_unavailable",
            "权威审计不可用：策略动作失败关闭，未发布。",
        )
    }
}

impl IntoResponse for Problem {
    fn into_response(self) -> Response {
        (self.status, Json(self.body)).into_response()
    }
}
