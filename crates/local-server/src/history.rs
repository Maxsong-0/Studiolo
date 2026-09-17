//! L0 查询只接收路径、版本及不透明游标；不暴露私有存储路径或安装能力。
mod response;
use crate::{AppState, CanonicalSource, editing::Problem};
use axum::{
    Json,
    extract::{Query, Request, State, rejection::QueryRejection},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use studiolo_core::{VaultPath, VaultUuid};
use studiolo_vault_broker::BrokerError;

pub(crate) const PATH: &str = "/api/v1/history";
pub(crate) const FILE: &str = "/api/v1/history/file";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PageQuery {
    vault_uuid: VaultUuid,
    path: VaultPath,
    #[serde(default = "default_limit")]
    limit: usize,
    cursor: Option<String>,
}
fn default_limit() -> usize {
    20
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FileQuery {
    vault_uuid: VaultUuid,
    path: VaultPath,
    revision: String,
}

fn valid_path(path: &VaultPath) -> bool {
    path.as_str().len() <= 4096 && path.as_str().to_lowercase().ends_with(".md")
}
fn bad_query() -> Response {
    Problem::new(
        StatusCode::BAD_REQUEST,
        "invalid_history_query",
        "需要有效且唯一的库、Markdown路径和历史版本。",
    )
    .into_response()
}

pub(crate) async fn page(
    State(state): State<AppState>,
    query: Result<Query<PageQuery>, QueryRejection>,
    mut request: Request,
) -> Response {
    let Ok(Query(query)) = query else {
        return bad_query();
    };
    if !valid_path(&query.path)
        || !(1..=50).contains(&query.limit)
        || query.cursor.as_ref().is_some_and(|cursor| {
            cursor.is_empty()
                || cursor.len() > 1024
                || !cursor.is_ascii()
                || cursor.chars().any(char::is_control)
        })
    {
        return bad_query();
    }
    let Some(CanonicalSource::Writer(owner)) = state.canonical else {
        return Problem::unavailable().into_response();
    };
    let Some(auth) = state.sessions else {
        return Problem::unavailable().into_response();
    };
    let principal = match auth.content_reader(&mut request) {
        Ok(principal) => principal,
        Err(response) => return *response,
    };
    let permit = match state.queries.try_acquire_owned() {
        Ok(permit) => permit,
        Err(_) => return busy(),
    };
    match tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let page = owner.history_page(
            principal,
            query.vault_uuid,
            &query.path,
            query.limit,
            query.cursor.as_deref(),
        )?;
        Ok::<_, Problem>(response::page(query.vault_uuid, query.path, page))
    })
    .await
    {
        Ok(Ok(value)) => Json(value).into_response(),
        Ok(Err(problem)) => problem.into_response(),
        Err(_) => Problem::unavailable().into_response(),
    }
}

pub(crate) async fn file(
    State(state): State<AppState>,
    query: Result<Query<FileQuery>, QueryRejection>,
    mut request: Request,
) -> Response {
    let Ok(Query(query)) = query else {
        return bad_query();
    };
    if !valid_path(&query.path) {
        return bad_query();
    }
    let Ok(revision) = crate::editing::revision(&query.revision) else {
        return bad_query();
    };
    let Some(CanonicalSource::Writer(owner)) = state.canonical else {
        return Problem::unavailable().into_response();
    };
    let Some(auth) = state.sessions else {
        return Problem::unavailable().into_response();
    };
    let principal = match auth.content_reader(&mut request) {
        Ok(principal) => principal,
        Err(response) => return *response,
    };
    let permit = match state.queries.try_acquire_owned() {
        Ok(permit) => permit,
        Err(_) => return busy(),
    };
    match tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let value = owner.history_file(principal, query.vault_uuid, &query.path, revision)?;
        let content = String::from_utf8(value.content).map_err(|_| {
            Problem::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "invalid_document",
                "历史版本不是有效的UTF-8文档。",
            )
        })?;
        Ok::<_, Problem>(
            serde_json::json!({"vault_uuid":value.vault_uuid.to_string(),
            "path":value.path.to_string(),"revision":value.revision.to_hex(),"content":content}),
        )
    })
    .await
    {
        Ok(Ok(value)) => Json(value).into_response(),
        Ok(Err(problem)) => problem.into_response(),
        Err(_) => Problem::unavailable().into_response(),
    }
}

fn busy() -> Response {
    Problem::new(
        StatusCode::TOO_MANY_REQUESTS,
        "busy",
        "历史查询繁忙，请稍后重试。",
    )
    .into_response()
}
pub(crate) fn read_error(error: BrokerError) -> Problem {
    match error {
        BrokerError::InvalidHistoryCursor => Problem::new(
            StatusCode::BAD_REQUEST,
            "invalid_history_query",
            "历史游标无效，请从第一页重新读取。",
        ),
        BrokerError::HistoryChanged => Problem::new(
            StatusCode::CONFLICT,
            "history_changed",
            "历史列表已经变化，请从第一页重新读取。",
        ),
        BrokerError::NotFound(_) => Problem::new(
            StatusCode::NOT_FOUND,
            "history_not_found",
            "历史版本不存在或已按保留策略清理。",
        ),
        BrokerError::ReadLimitExceeded { .. } => Problem::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "too_large",
            "历史版本超出网页预览限制。",
        ),
        BrokerError::ProtectedPath(_)
        | BrokerError::UnsafeNode { .. }
        | BrokerError::PathCollision { .. } => Problem::forbidden(),
        _ => Problem::unavailable(),
    }
}
