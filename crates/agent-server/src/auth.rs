//! 类型 token 门禁：无 / 错 / 吊销 token 一律 401，不进入任何内容计算；通过后把类型放进请求扩展。
//! 身份库读不出来时令牌真假未知，回 503、不带 challenge（BUG-030），不能折成 401。

use std::sync::Arc;

use axum::{
    body::Body,
    extract::State,
    http::{Request, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use sha2::{Digest as _, Sha256};
use studiolo_supply::audit::{self, Actor};

use crate::state::AppState;

/// 已认证的类型；由中间件注入，handler 在 `http::request::Parts` 的 extensions 里读取。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentType(pub String);

/// 只在进程内传递的当前凭据摘要；不实现 Debug 或序列化。
#[derive(Clone)]
pub(crate) struct CredentialDigest(pub String);

/// 身份库读不出来时的拒绝原因；认证门、MCP 门禁与 HTTP 文本外壳共用一个审计类别。
pub(crate) const IDENTITY_UNAVAILABLE: &str = "identity_unavailable";

/// 身份库读不出来（SQLite 忙超时、I/O 错、私有存储复核失败）时的应答：503 且不带
/// `WWW-Authenticate`。凭据真假未知，回 401 会让客户端当成令牌失效、转去重新认证；
/// 正文只给固定类别，不回显底层错误或路径。
pub(crate) fn identity_unavailable() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        "身份校验暂不可用，请稍后重试",
    )
        .into_response()
}

/// 认证中间件。
pub async fn require_bearer(
    State(state): State<Arc<AppState>>,
    mut request: Request<Body>,
    next: Next,
) -> Response {
    let token = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let token = token.filter(|_| {
        request
            .headers()
            .get_all(header::AUTHORIZATION)
            .iter()
            .count()
            == 1
    });
    let authenticated = match token {
        Some(token) => match state.store.authenticate(token) {
            Ok(agent_type) => agent_type.map(|agent_type| {
                (
                    agent_type,
                    CredentialDigest(
                        Sha256::digest(token.as_bytes())
                            .iter()
                            .map(|byte| format!("{byte:02x}"))
                            .collect(),
                    ),
                )
            }),
            // 令牌真假未知：不能当成「没有这个令牌」回 401。
            Err(_) => {
                deny(&state, &request, IDENTITY_UNAVAILABLE);
                return identity_unavailable();
            }
        },
        None => None,
    };
    match authenticated {
        Some((agent_type, digest)) => {
            request.extensions_mut().insert(AgentType(agent_type));
            request.extensions_mut().insert(digest);
            next.run(request).await
        }
        None => {
            deny(&state, &request, "unauthenticated");
            (
                StatusCode::UNAUTHORIZED,
                [(header::WWW_AUTHENTICATE, "Bearer realm=\"studiolo-agent\"")],
            )
                .into_response()
        }
    }
}

/// 拒绝本身也进审计；账本不可用时进入降级窗口，但拒绝照常返回。
fn deny(state: &AppState, request: &Request<Body>, reason: &str) {
    if let Ok(event) = audit::action_denied(
        Actor::anonymous(),
        if request.uri().path() == "/mcp" {
            "mcp.request"
        } else {
            "http.request"
        },
        reason,
    ) {
        let _ = state.audit(event);
    }
}
