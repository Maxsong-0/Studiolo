//! 类型 token 门禁：无 / 错 / 吊销 token 一律 401，不进入任何内容计算；通过后把类型放进请求扩展。

use std::sync::Arc;

use axum::{
    body::Body,
    extract::State,
    http::{Request, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use studiolo_supply::audit::{self, Actor};

use crate::state::AppState;

/// 已认证的类型；由中间件注入，handler 在 `http::request::Parts` 的 extensions 里读取。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentType(pub String);

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
        Some(token) => state.store.authenticate(token).unwrap_or(None),
        None => None,
    };
    match authenticated {
        Some(agent_type) => {
            request.extensions_mut().insert(AgentType(agent_type));
            next.run(request).await
        }
        None => {
            // 拒绝本身也进审计；账本不可用时进入降级窗口，但 401 照常返回。
            if let Ok(event) = audit::action_denied(
                Actor::anonymous(),
                if request.uri().path() == "/mcp" {
                    "mcp.request"
                } else {
                    "http.request"
                },
                "unauthenticated",
            ) {
                let _ = state.audit(event);
            }
            (
                StatusCode::UNAUTHORIZED,
                [(header::WWW_AUTHENTICATE, "Bearer realm=\"studiolo-agent\"")],
            )
                .into_response()
        }
    }
}
