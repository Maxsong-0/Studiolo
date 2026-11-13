//! 仅人侧附件读取：清单准入、Broker 稳定读取和惰性下载响应共同构成边界。

mod media;
mod stream;

use axum::{
    body::Body,
    extract::{Query, State, rejection::QueryRejection},
    http::{HeaderValue, StatusCode, header},
    response::Response,
};
use serde::Deserialize;
use studiolo_core::VaultPath;
use studiolo_vault_broker::BrokerError;
use studiolo_workbench::TreeKind;

use crate::{ApiError, AppState, LocalError, Source, queries};

const MAX_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Clone)]
pub(crate) struct DownloadPolicy;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FileQuery {
    path: String,
    generation: u64,
}

pub(crate) async fn file(
    State(state): State<AppState>,
    query: Result<Query<FileQuery>, QueryRejection>,
) -> Result<Response, ApiError> {
    let Query(query) = query.map_err(|_| bad_query())?;
    if query.path.len() > 4096 || query.path.split('/').any(|part| part.starts_with('.')) {
        return Err(bad_query());
    }
    let path = VaultPath::parse(&query.path).map_err(|_| bad_query())?;
    let generation = queries::at_generation(&state, query.generation)?;
    let Some((TreeKind::Attachment, size)) = generation.tree.file(path.as_str()) else {
        return Err(ApiError(
            StatusCode::NOT_FOUND,
            "附件不在当前快照中。".into(),
        ));
    };
    if size > MAX_BYTES {
        return Err(too_large());
    }
    let slots = state.queries.clone();
    queries::run_leased_query(&slots, move |permit| {
        let content = match &state.source {
            Source::Replica { vault, .. } => {
                vault
                    .read_file(&path, MAX_BYTES)
                    .map_err(read_error)?
                    .content
            }
            Source::Writer(workbench) => {
                workbench
                    .read_file(&path, MAX_BYTES)
                    .map_err(|error| match error {
                        LocalError::Read(error) => read_error(error),
                        _ => ApiError::unavailable(),
                    })?
                    .content
            }
        };
        // 不提供扫描时字节的承诺；同大小改动允许，大小或代次变化要求重新取视图。
        if content.len() as u64 != size {
            return Err(changed());
        }
        queries::at_generation(&state, query.generation)?;
        let mime = media::content_type(path.as_str(), &content);
        let disposition = HeaderValue::from_str(&media::disposition(path.as_str()))
            .map_err(|_| ApiError::unavailable())?;
        let mut response = Response::new(Body::from_stream(stream::Download::new(content, permit)));
        let headers = response.headers_mut();
        headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(mime));
        headers.insert(header::CONTENT_LENGTH, HeaderValue::from(size));
        headers.insert(header::CONTENT_DISPOSITION, disposition);
        headers.insert(header::ACCEPT_RANGES, HeaderValue::from_static("none"));
        response.extensions_mut().insert(DownloadPolicy);
        Ok(response)
    })
    .await
}

fn bad_query() -> ApiError {
    ApiError(
        StatusCode::BAD_REQUEST,
        "需要唯一且有效的可见附件路径与代次。".into(),
    )
}

fn changed() -> ApiError {
    ApiError(
        StatusCode::CONFLICT,
        "附件已变化，请刷新工作台后重试。".into(),
    )
}

fn too_large() -> ApiError {
    ApiError(
        StatusCode::PAYLOAD_TOO_LARGE,
        "附件超出网页读取上限，请在同步副本中打开。".into(),
    )
}

fn read_error(error: BrokerError) -> ApiError {
    match error {
        BrokerError::NotFound(_) => ApiError(StatusCode::NOT_FOUND, "附件不存在。".into()),
        BrokerError::ProtectedPath(_)
        | BrokerError::UnsafeNode { .. }
        | BrokerError::PathCollision { .. } => {
            ApiError(StatusCode::FORBIDDEN, "附件路径不允许读取。".into())
        }
        BrokerError::ReadLimitExceeded { .. } => too_large(),
        BrokerError::UnstableRead(_) => changed(),
        _ => ApiError::unavailable(),
    }
}
