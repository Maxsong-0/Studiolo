//! L2 内容变更入口；分页快照不授予权限，每次请求仍重验原会话。
#[cfg(test)]
mod tests;
use crate::{AppState, CanonicalSource, editing::Problem};
use axum::{
    Json,
    extract::{Query, Request, State, rejection::QueryRejection},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use studiolo_core::{VaultPath, VaultUuid};
pub(crate) const PATH: &str = "/api/v1/changes";
pub(crate) const SEEN: &str = "/api/v1/changes/seen";
pub(crate) const HEAD: &str = "/api/v1/changes/head";
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PageQuery {
    pub(crate) vault_uuid: Option<VaultUuid>,
    pub(crate) project: Option<VaultPath>,
    pub(crate) before: Option<String>,
    #[serde(default = "limit")]
    pub(crate) limit: usize,
}
/// 只问水位（T-210）：库身份可以不带，带了就核对。
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HeadQuery {
    pub(crate) vault_uuid: Option<VaultUuid>,
}
fn limit() -> usize {
    50
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SeenInput {
    pub(crate) vault_uuid: VaultUuid,
    pub(crate) project: Option<VaultPath>,
    pub(crate) snapshot: String,
    pub(crate) request_id: String,
}
pub(crate) fn scope(project: Option<&VaultPath>) -> bool {
    project.is_none_or(|p| {
        let mut parts = p.as_str().split('/');
        parts.next() == Some("项目")
            && parts.next().is_some()
            && parts.next().is_none()
            && !p.is_protected()
            && p.as_str().len() <= 256
    })
}
pub(crate) fn token(value: &str) -> bool {
    value.len() == 36 && value.parse::<VaultUuid>().is_ok()
}
pub(crate) fn parse_seen(bytes: &[u8]) -> Result<SeenInput, ()> {
    let input: SeenInput = serde_json::from_slice(bytes).map_err(|_| ())?;
    if !scope(input.project.as_ref()) || !token(&input.snapshot) || !token(&input.request_id) {
        return Err(());
    }
    Ok(input)
}
pub(crate) fn invalid() -> Problem {
    Problem::new(
        StatusCode::BAD_REQUEST,
        "invalid_changes_query",
        "变更查询无效，请重新打开列表。",
    )
}
pub(crate) fn expired() -> Problem {
    Problem::new(
        StatusCode::CONFLICT,
        "changes_expired",
        "变更快照已失效，请从第一页重新读取。",
    )
}
fn busy() -> Response {
    Problem::new(
        StatusCode::TOO_MANY_REQUESTS,
        "busy",
        "变更查询繁忙，请稍后重试。",
    )
    .into_response()
}
pub(crate) async fn page(
    State(state): State<AppState>,
    query: Result<Query<PageQuery>, QueryRejection>,
    mut request: Request,
) -> Response {
    let Ok(Query(query)) = query else {
        return invalid().into_response();
    };
    if !scope(query.project.as_ref())
        || !(1..=100).contains(&query.limit)
        || query.before.as_deref().is_some_and(|v| !token(v))
    {
        return invalid().into_response();
    }
    let Some(CanonicalSource::Writer(owner)) = state.canonical else {
        return Problem::unavailable().into_response();
    };
    let Some(auth) = state.sessions else {
        return Problem::unavailable().into_response();
    };
    let principal = match auth.content_reader(&mut request) {
        Ok(v) => v,
        Err(r) => return *r,
    };
    let permit = match state.queries.try_acquire_owned() {
        Ok(v) => v,
        Err(_) => return busy(),
    };
    match tokio::task::spawn_blocking(move || {
        let _permit = permit;
        owner.changes_page(&principal, query)
    })
    .await
    {
        Ok(Ok(v)) => Json(v).into_response(),
        Ok(Err(e)) => e.into_response(),
        Err(_) => Problem::unavailable().into_response(),
    }
}
/// 当前水位，不带任何条目：页面拿它判断「该不该重读列表」。每次请求照常重验会话和根绑定。
pub(crate) async fn head(
    State(state): State<AppState>,
    query: Result<Query<HeadQuery>, QueryRejection>,
    mut request: Request,
) -> Response {
    let Ok(Query(query)) = query else {
        return invalid().into_response();
    };
    let Some(CanonicalSource::Writer(owner)) = state.canonical else {
        return Problem::unavailable().into_response();
    };
    let Some(auth) = state.sessions else {
        return Problem::unavailable().into_response();
    };
    let principal = match auth.content_reader(&mut request) {
        Ok(v) => v,
        Err(r) => return *r,
    };
    let permit = match state.queries.try_acquire_owned() {
        Ok(v) => v,
        Err(_) => return busy(),
    };
    match tokio::task::spawn_blocking(move || {
        let _permit = permit;
        owner.changes_head(&principal, query.vault_uuid)
    })
    .await
    {
        Ok(Ok(v)) => Json(v).into_response(),
        Ok(Err(e)) => e.into_response(),
        Err(_) => Problem::unavailable().into_response(),
    }
}
pub(crate) async fn mark(State(state): State<AppState>, mut request: Request) -> Response {
    let Some(CanonicalSource::Writer(owner)) = state.canonical else {
        return Problem::unavailable().into_response();
    };
    let Some(auth) = state.sessions else {
        return Problem::unavailable().into_response();
    };
    let (principal, input) = match auth.changes_seen(&mut request) {
        Ok(v) => v,
        Err(r) => return *r,
    };
    let permit = match state.queries.try_acquire_owned() {
        Ok(v) => v,
        Err(_) => return busy(),
    };
    match tokio::task::spawn_blocking(move || {
        let _permit = permit;
        owner.mark_changes_seen(&principal, input)
    })
    .await
    {
        Ok(Ok(v)) => Json(v).into_response(),
        Ok(Err(e)) => e.into_response(),
        Err(_) => Problem::unavailable().into_response(),
    }
}
pub(crate) fn check(
    owner: &crate::CanonicalWorkbench,
    principal: &crate::auth_http::ContentPrincipal,
) -> Result<(), Problem> {
    owner.check_changes_access(principal)
}
pub(crate) fn actor(principal: &crate::auth_http::ContentPrincipal) -> String {
    use sha2::{Digest, Sha256};
    let (issuer, subject) = principal.actor();
    let raw = serde_json::to_vec(&(issuer, subject)).expect("主体字符串可编码");
    Sha256::digest(raw)
        .iter()
        .map(|v| format!("{v:02x}"))
        .collect()
}
