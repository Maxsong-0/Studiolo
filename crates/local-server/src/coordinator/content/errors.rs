use crate::editing::{Problem, Replace};
use axum::http::StatusCode;
use studiolo_core::Revision;
use studiolo_vault_broker::BrokerError;
use studiolo_vault_index::{InMemoryVaultIndex, MarkdownDocument, VaultIndex};

pub(super) fn valid_document(path: &studiolo_core::VaultPath, content: &str) -> bool {
    let mut index = InMemoryVaultIndex::new();
    index
        .rebuild(&[MarkdownDocument::new(path.clone(), content)])
        .is_ok()
        && index
            .document(path)
            .ok()
            .flatten()
            .is_some_and(|doc| doc.frontmatter.warnings.is_empty())
}
pub(crate) fn session_error(error: studiolo_auth::SessionError) -> Problem {
    match error {
        studiolo_auth::SessionError::Rejected => Problem::new(
            StatusCode::UNAUTHORIZED,
            "session_required",
            "应用会话已失效，请保留草稿并重新登录。",
        ),
        _ => Problem::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "auth_unavailable",
            "身份存储暂不可用，未执行保存；请保留草稿稍后重试。",
        ),
    }
}
pub(super) fn audit_unavailable() -> Problem {
    Problem::new(
        StatusCode::SERVICE_UNAVAILABLE,
        "audit_unavailable",
        "权威审计尚未就绪，未执行本次写入；请保留草稿。",
    )
}
pub(super) fn read_error(error: BrokerError) -> Problem {
    match error {
        BrokerError::ReadLimitExceeded { .. } => Problem::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "too_large",
            "文档超出网页编辑限制。",
        ),
        _ => Problem::unavailable(),
    }
}
pub(super) fn mutation_error(error: BrokerError, input: &Replace) -> Problem {
    match error {
        BrokerError::RevisionConflict { actual, .. } => Problem {
            status: StatusCode::CONFLICT,
            body: serde_json::json!({"code":"revision_conflict", "error":"磁盘版本已变化，请比较双方内容。",
                "vault_uuid":input.vault.to_string(),"path":input.path.to_string(),
                "expected_revision":input.expected.to_hex(),"actual_revision":actual.map(Revision::to_hex)}),
        },
        BrokerError::IdempotencyConflict => Problem::new(
            StatusCode::CONFLICT,
            "idempotency_conflict",
            "此请求标识已用于不同内容，请保留草稿并核对。",
        ),
        BrokerError::MutationCancelled => Problem::new(
            StatusCode::CONFLICT,
            "mutation_cancelled",
            "原请求已由受信任流程取消，请重新核对版本。",
        ),
        BrokerError::MutationIndeterminate { .. } => Problem::unknown(),
        BrokerError::ProtectedPath(_)
        | BrokerError::UnsafeNode { .. }
        | BrokerError::PathCollision { .. } => Problem::forbidden(),
        _ => Problem::unknown(),
    }
}
