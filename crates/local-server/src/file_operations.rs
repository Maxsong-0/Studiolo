//! 独立文件操作HTTP；配置关闭也返回稳定disabled，不从客户端初始化存储。
pub(crate) mod problem;
mod request;
mod response;
use crate::{AppState, CanonicalSource};
use axum::{
    Json, Router,
    extract::{Query, Request, State, rejection::QueryRejection},
    response::{IntoResponse, Response},
    routing::get,
};
use request::PageQuery;
pub(crate) use request::{Input, Kind, parse};

pub(crate) const PATH: &str = "/api/v1/file-operations";
pub(crate) const TOMBSTONES: &str = "/api/v1/tombstones";
pub(crate) const JSON_LIMIT: usize = 32 * 1024;

pub(crate) fn available(state: &AppState) -> bool {
    matches!(state.canonical, Some(CanonicalSource::Writer(_))) && state.sessions.is_some()
}
pub(crate) fn routes(routes: Router<AppState>, state: &AppState) -> Router<AppState> {
    if available(state) {
        routes
            .route(PATH, get(capability).post(mutate))
            .route(TOMBSTONES, get(page))
    } else {
        routes
    }
}

async fn capability(State(state): State<AppState>, mut request: Request) -> Response {
    let Some(CanonicalSource::Writer(owner)) = state.canonical else {
        return problem::disabled().into_response();
    };
    let Some(auth) = state.sessions else {
        return problem::disabled().into_response();
    };
    let principal = match auth.content_reader(&mut request) {
        Ok(value) => value,
        Err(response) => return *response,
    };
    let permit = match state.queries.try_acquire_owned() {
        Ok(value) => value,
        Err(_) => return problem::busy().into_response(),
    };
    match tokio::task::spawn_blocking(move || {
        let _permit = permit;
        owner.file_operation_capability(principal)
    })
    .await
    {
        Ok(Ok((vault, rename))) => {
            Json(serde_json::json!({"vault_uuid":vault.to_string(),"enabled":true,"rename_enabled":rename})).into_response()
        }
        Ok(Err(error)) => error.into_response(),
        Err(_) => problem::unavailable().into_response(),
    }
}

async fn mutate(State(state): State<AppState>, mut request: Request) -> Response {
    let Some(CanonicalSource::Writer(owner)) = state.canonical else {
        return problem::disabled().into_response();
    };
    let Some(auth) = state.sessions else {
        return problem::disabled().into_response();
    };
    let (principal, input) = match auth.file_operation(&mut request) {
        Ok(value) => value,
        Err(response) => return *response,
    };
    let permit = match state.queries.try_acquire_owned() {
        Ok(value) => value,
        Err(_) => return problem::busy().into_response(),
    };
    match tokio::task::spawn_blocking(move || {
        let _permit = permit;
        owner.human_file_operation(principal, input)
    })
    .await
    {
        Ok(Ok(receipt)) => Json(response::saved(receipt)).into_response(),
        Ok(Err(error)) => error.into_response(),
        Err(_) => problem::unknown().into_response(),
    }
}

async fn page(
    State(state): State<AppState>,
    query: Result<Query<PageQuery>, QueryRejection>,
    mut request: Request,
) -> Response {
    let Ok(Query(query)) = query else {
        return problem::invalid_query().into_response();
    };
    if !query.valid() {
        return problem::invalid_query().into_response();
    }
    let Some(CanonicalSource::Writer(owner)) = state.canonical else {
        return problem::disabled().into_response();
    };
    let Some(auth) = state.sessions else {
        return problem::disabled().into_response();
    };
    let principal = match auth.content_reader(&mut request) {
        Ok(value) => value,
        Err(response) => return *response,
    };
    let permit = match state.queries.try_acquire_owned() {
        Ok(value) => value,
        Err(_) => return problem::busy().into_response(),
    };
    match tokio::task::spawn_blocking(move || {
        let _permit = permit;
        owner.tombstones(
            principal,
            query.vault_uuid,
            query.limit,
            query.cursor.as_deref(),
        )
    })
    .await
    {
        Ok(Ok(page)) => Json(response::page(page)).into_response(),
        Ok(Err(error)) => error.into_response(),
        Err(_) => problem::unavailable().into_response(),
    }
}

#[cfg(test)]
#[path = "../tests/file_operations/request.rs"]
mod tests;
