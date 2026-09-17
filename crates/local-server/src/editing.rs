//! 人侧正文写入入口；HTTP DTO 与 Broker mutation 不共享反序列化能力。

pub(crate) mod audit;
mod request;

use crate::{AppState, CanonicalSource, CanonicalWorkbench};
use axum::{
    Json,
    extract::{Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;
use studiolo_core::{Revision, VaultPath, VaultUuid};
use studiolo_vault_broker::MutationReceipt;

pub(crate) use request::revision;
pub(crate) use request::{Replace, parse};
pub(crate) const PATH: &str = "/api/v1/mutations";
pub(crate) const MAX_BYTES: usize = 256 * 1024;
pub(crate) const JSON_LIMIT: usize = MAX_BYTES * 6 + 16 * 1024;

pub(crate) fn digest(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[derive(Serialize)]
pub(crate) struct Editing {
    allowed: bool,
    reason: Option<&'static str>,
    max_bytes: usize,
}

pub(crate) fn enabled(state: &AppState) -> bool {
    matches!(&state.canonical, Some(CanonicalSource::Writer(w)) if w.editing_enabled())
}

impl CanonicalWorkbench {
    pub(crate) fn editing_enabled(&self) -> bool {
        self.core.roots.human_content().is_some() && self.core.content_audit.is_some()
    }

    pub(crate) fn editing(&self, vault: VaultUuid, path: &VaultPath, content: &str) -> Editing {
        let reason = if !self.editing_enabled() {
            Some("read_only")
        } else if self
            .core
            .content_restore_pending
            .load(std::sync::atomic::Ordering::Acquire)
            || self
                .core
                .file_operation_pending
                .load(std::sync::atomic::Ordering::Acquire)
        {
            Some("audit_unavailable")
        } else if self.core.roots.human_content().is_none_or(|scope| {
            !scope.allows(vault, path) || !scope.preserves_generated(path, content, content)
        }) {
            Some("protected")
        } else if content.len() > MAX_BYTES {
            Some("too_large")
        } else if self.revalidate().is_err() {
            Some("stale")
        } else {
            None
        };
        Editing {
            allowed: reason.is_none(),
            reason,
            max_bytes: MAX_BYTES,
        }
    }
}

pub(crate) fn read_only() -> Editing {
    Editing {
        allowed: false,
        reason: Some("read_only"),
        max_bytes: MAX_BYTES,
    }
}

pub(crate) async fn handle(State(state): State<AppState>, mut request: Request) -> Response {
    let Some(CanonicalSource::Writer(workbench)) = state.canonical else {
        return Problem::unavailable().into_response();
    };
    let Some(auth) = state.sessions else {
        return Problem::unavailable().into_response();
    };
    let (principal, input) = match auth.content(&mut request) {
        Ok(value) => value,
        Err(response) => return *response,
    };
    let permit = match state.queries.try_acquire_owned() {
        Ok(permit) => permit,
        Err(_) => {
            return Problem::new(
                StatusCode::TOO_MANY_REQUESTS,
                "busy",
                "保存队列已满，请稍后重试。",
            )
            .into_response();
        }
    };
    let result = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        workbench.replace(principal, input)
    })
    .await;
    match result {
        Ok(Ok(receipt)) => Json(Saved::from(receipt)).into_response(),
        Ok(Err(error)) => error.into_response(),
        Err(_) => Problem::unknown().into_response(),
    }
}

#[derive(Serialize)]
struct Saved {
    status: &'static str,
    receipt: Receipt,
}
#[derive(Serialize)]
struct Receipt {
    path: String,
    previous_revision: Option<String>,
    new_revision: String,
}
impl From<MutationReceipt> for Saved {
    fn from(receipt: MutationReceipt) -> Self {
        Self {
            status: "committed",
            receipt: Receipt {
                path: receipt.path.to_string(),
                previous_revision: receipt.previous_revision.map(Revision::to_hex),
                new_revision: receipt.revision.to_hex(),
            },
        }
    }
}

pub(crate) struct Problem {
    pub(crate) status: StatusCode,
    pub(crate) body: serde_json::Value,
}
impl Problem {
    pub(crate) fn new(status: StatusCode, code: &'static str, error: &'static str) -> Self {
        Self {
            status,
            body: serde_json::json!({"code":code,"error":error}),
        }
    }
    pub(crate) fn unavailable() -> Self {
        Self::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "unavailable",
            "正文写入暂不可用。",
        )
    }
    pub(crate) fn unknown() -> Self {
        Self::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "outcome_unknown",
            "保存结果尚不能确认，请保留草稿并核对原请求。",
        )
    }
    pub(crate) fn stale() -> Self {
        Self::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "stale",
            "完整视图尚未追平，请重新核验后保存。",
        )
    }
    pub(crate) fn forbidden() -> Self {
        Self::new(
            StatusCode::FORBIDDEN,
            "protected",
            "此路径或生成区不在允许编辑的范围内。",
        )
    }
}
impl IntoResponse for Problem {
    fn into_response(self) -> Response {
        (self.status, Json(self.body)).into_response()
    }
}
