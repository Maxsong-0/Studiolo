//! 人侧单页权威读取，只依赖 Broker 窄读能力，不获得文件系统写入口。

use crate::{ApiError, AppState, CanonicalSource, LocalError, queries::run_query};
use axum::{
    Json,
    extract::{Query, State, rejection::QueryRejection},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use studiolo_core::VaultPath;
use studiolo_vault_broker::BrokerError;

pub(crate) const PATH: &str = "/api/v1/file";
const MAX_BYTES: u64 = 1024 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FileQuery {
    path: String,
}

#[derive(Serialize)]
pub(crate) struct FileResponse {
    vault_uuid: String,
    path: String,
    content: String,
    revision: String,
}

pub(crate) async fn file(
    State(state): State<AppState>,
    query: Result<Query<FileQuery>, QueryRejection>,
) -> Result<Json<FileResponse>, ApiError> {
    let Query(query) = query.map_err(|_| bad_query())?;
    if query.path.len() > 4096
        || !query
            .path
            .rsplit_once('.')
            .is_some_and(|(_, extension)| extension.eq_ignore_ascii_case("md"))
    {
        return Err(bad_query());
    }
    let path = VaultPath::parse(query.path).map_err(|_| bad_query())?;
    let vault = state.canonical.clone().ok_or_else(ApiError::unavailable)?;
    run_query(&state.queries, move || {
        let note =
            match vault {
                CanonicalSource::Reader(vault) => {
                    vault.read_file(&path, MAX_BYTES).map_err(read_error)?
                }
                CanonicalSource::Writer(workbench) => workbench
                    .read_file(&path, MAX_BYTES)
                    .map_err(|error| match error {
                        LocalError::Read(error) => read_error(error),
                        _ => ApiError::unavailable(),
                    })?,
            };
        let content = String::from_utf8(note.content).map_err(|_| {
            ApiError(
                StatusCode::UNPROCESSABLE_ENTITY,
                "文档不是有效 UTF-8。".into(),
            )
        })?;
        Ok(Json(FileResponse {
            vault_uuid: note.vault_uuid.to_string(),
            path: note.path.to_string(),
            content,
            revision: note.revision.to_hex(),
        }))
    })
    .await
}

fn bad_query() -> ApiError {
    ApiError(
        StatusCode::BAD_REQUEST,
        "需要唯一且有效的 Markdown 文档路径。".into(),
    )
}

fn read_error(error: BrokerError) -> ApiError {
    match error {
        BrokerError::NotFound(_) => ApiError(StatusCode::NOT_FOUND, "文档不存在。".into()),
        BrokerError::ProtectedPath(_)
        | BrokerError::UnsafeNode { .. }
        | BrokerError::PathCollision { .. } => {
            ApiError(StatusCode::FORBIDDEN, "文档路径不允许读取。".into())
        }
        BrokerError::ReadLimitExceeded { .. } => ApiError(
            StatusCode::PAYLOAD_TOO_LARGE,
            "文档超出单页读取限制。".into(),
        ),
        BrokerError::UnstableRead(_) => {
            ApiError(StatusCode::CONFLICT, "文档正在变化，请重新读取。".into())
        }
        _ => ApiError(
            StatusCode::SERVICE_UNAVAILABLE,
            "权威文档暂不可用，未返回旧内容。".into(),
        ),
    }
}
