//! 先拒绝重复句柄、无会话协议与非初始化首包，避免绕开有状态配额。
use axum::{
    body::{Body, to_bytes},
    http::{HeaderMap, Request, StatusCode},
};

pub(super) const SESSION: &str = "mcp-session-id";
pub(super) const MAX_BODY: usize = 4 * 1024 * 1024;

pub(super) fn session(headers: &HeaderMap) -> Result<Option<String>, StatusCode> {
    let values = headers.get_all(SESSION).iter().collect::<Vec<_>>();
    match values.as_slice() {
        [] => Ok(None),
        [value] => value
            .to_str()
            .ok()
            .filter(|value| {
                !value.is_empty()
                    && value.len() <= 128
                    && value
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            })
            .map(|value| Some(value.to_owned()))
            .ok_or(StatusCode::BAD_REQUEST),
        _ => Err(StatusCode::BAD_REQUEST),
    }
}

pub(super) fn legacy_headers(headers: &HeaderMap) -> Result<(), StatusCode> {
    let mut values = headers.get_all("mcp-protocol-version").iter();
    let version = values.next();
    if values.next().is_some()
        || version.is_some_and(|value| {
            !matches!(
                value.to_str(),
                Ok("2025-11-25" | "2025-06-18" | "2025-03-26")
            )
        })
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    Ok(())
}

pub(super) async fn post(
    request: Request<Body>,
    initialize: bool,
) -> Result<Request<Body>, StatusCode> {
    let (parts, body) = request.into_parts();
    let bytes = to_bytes(body, MAX_BODY)
        .await
        .map_err(|_| StatusCode::PAYLOAD_TOO_LARGE)?;
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|_| StatusCode::BAD_REQUEST)?;
    let method = value.get("method").and_then(serde_json::Value::as_str);
    // 首次 initialize 仍交 SDK 协商旧版；后续请求不得切换为无会话生命周期或再次注册实例。
    if initialize && method != Some("initialize")
        || !initialize && matches!(method, Some("initialize" | "server/discover"))
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    if !initialize
        && value
            .pointer("/params/_meta")
            .and_then(serde_json::Value::as_object)
            .is_some_and(|meta| {
                meta.keys()
                    .any(|key| key.starts_with("io.modelcontextprotocol/"))
            })
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    Ok(Request::from_parts(parts, Body::from(bytes)))
}
