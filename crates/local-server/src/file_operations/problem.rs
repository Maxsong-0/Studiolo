//! 错误只陈述已确认的阶段；缺失或当前同内容不补成成功证明。
use crate::editing::Problem;
use axum::http::StatusCode;
use studiolo_vault_broker::{BrokerError, FileOperationOutcome, FileOperationReceipt};

pub(crate) fn disabled() -> Problem {
    Problem::new(
        StatusCode::FORBIDDEN,
        "disabled",
        "文件删除与墓碑恢复尚未启用。",
    )
}
pub(crate) fn rename_disabled() -> Problem {
    Problem::new(StatusCode::FORBIDDEN, "disabled", "文件重命名尚未启用。")
}
pub(crate) fn rename_content() -> Problem {
    Problem::new(
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
        "unsupported_rename_content",
        "此重命名范围只批准完整有效且不含非文本控制字符的 UTF-8 Markdown；不能转换正文或附件类型。",
    )
}
pub(crate) fn unknown() -> Problem {
    Problem::new(
        StatusCode::SERVICE_UNAVAILABLE,
        "outcome_unknown",
        "原文件操作结果尚未确认；请保留原请求标识核对，不要换键重试。",
    )
}
pub(crate) fn unavailable() -> Problem {
    Problem::new(
        StatusCode::SERVICE_UNAVAILABLE,
        "unavailable",
        "文件操作暂不可用。",
    )
}
pub(crate) fn audit_unavailable() -> Problem {
    Problem::new(
        StatusCode::SERVICE_UNAVAILABLE,
        "audit_unavailable",
        "权威审计尚未就绪；本次未开始新的文件安装。",
    )
}
pub(crate) fn busy() -> Problem {
    Problem::new(
        StatusCode::TOO_MANY_REQUESTS,
        "busy",
        "文件操作队列已满，请稍后重试。",
    )
}
pub(crate) fn invalid_query() -> Problem {
    Problem::new(
        StatusCode::BAD_REQUEST,
        "invalid_tombstone_query",
        "墓碑查询参数无效，请从第一页重新查询。",
    )
}

pub(crate) fn receipt(value: FileOperationReceipt) -> Result<FileOperationReceipt, Problem> {
    match value.outcome {
        FileOperationOutcome::Committed => Ok(value),
        FileOperationOutcome::Rejected => Err(Problem::new(
            StatusCode::CONFLICT,
            "file_operation_conflict",
            "原文件条件不匹配；本请求已固定为未执行，请重新核对路径。",
        )),
        FileOperationOutcome::Cancelled => Err(Problem::new(
            StatusCode::CONFLICT,
            "file_operation_cancelled",
            "原文件操作已经取消；请重新核对后再发起新的操作。",
        )),
        FileOperationOutcome::Indeterminate => Err(unknown()),
    }
}

/// 只在已确认没有新持久未决槽时使用；在途错误一律由 owner 升为 unknown。
pub(crate) fn before_issue(error: BrokerError) -> Problem {
    match error {
        BrokerError::IdempotencyConflict => Problem::new(
            StatusCode::CONFLICT,
            "idempotency_conflict",
            "原请求标识已绑定其他条件，请核对原请求。",
        ),
        BrokerError::ProtectedPath(_)
        | BrokerError::UnsafeNode { .. }
        | BrokerError::PathCollision { .. } => Problem::forbidden(),
        BrokerError::WriterLimitExceeded => Problem::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "budget_exceeded",
            "文件操作的保留或容量预算不足；未开始新的可见变更。",
        ),
        BrokerError::ReadLimitExceeded { .. } => Problem::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "too_large",
            "对象超过本次批准的文件操作限额。",
        ),
        BrokerError::CaptureChanged | BrokerError::UnstableRead(_) => Problem::stale(),
        BrokerError::MutationUnresolved => unknown(),
        _ => unavailable(),
    }
}

pub(crate) fn query(error: BrokerError) -> Problem {
    match error {
        BrokerError::HistoryChanged => Problem::new(
            StatusCode::CONFLICT,
            "tombstones_changed",
            "最近删除列表已变化，请从第一页重新查询。",
        ),
        BrokerError::InvalidHistoryCursor => invalid_query(),
        _ => before_issue(error),
    }
}
