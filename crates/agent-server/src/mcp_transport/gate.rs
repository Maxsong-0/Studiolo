//! 所有 HTTP 方法先匹配预占中的凭据与 SID，再交给 rmcp。
//! 状态码按 MCP 2025-11-25 会话管理：会话不可用回 404、缺会话号回 400；401 只给凭据本身失效；
//! 身份库读不出来回 503，会话留着下一次再判（BUG-030）。
use super::{
    TransportState, Validity, body, input,
    manager::{REQUEST_BINDING, Reservation},
    pool::ReserveError,
};
use crate::auth::{AgentType, CredentialDigest, IDENTITY_UNAVAILABLE};
use axum::{
    body::Body,
    extract::State,
    http::{HeaderValue, Method, Request, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use std::{sync::Arc, time::Instant};

fn failure(status: StatusCode) -> Response {
    let mut response = (status, "MCP 请求或会话不可用").into_response();
    if status == StatusCode::UNAUTHORIZED {
        response.headers_mut().insert(
            header::WWW_AUTHENTICATE,
            HeaderValue::from_static("Bearer realm=\"studiolo-agent\""),
        );
    }
    if status == StatusCode::TOO_MANY_REQUESTS {
        response
            .headers_mut()
            .insert(header::RETRY_AFTER, HeaderValue::from_static("60"));
    }
    response
}

pub(super) async fn gate(
    State(state): State<Arc<TransportState>>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let Some(AgentType(owner)) = request.extensions().get::<AgentType>().cloned() else {
        return failure(StatusCode::UNAUTHORIZED);
    };
    let Some(CredentialDigest(credential)) =
        request.extensions().get::<CredentialDigest>().cloned()
    else {
        return failure(StatusCode::UNAUTHORIZED);
    };
    let permit = match state.requests.clone().try_acquire_owned() {
        Ok(permit) => permit,
        Err(_) => {
            state.deny(&owner, "request_capacity");
            return failure(StatusCode::TOO_MANY_REQUESTS);
        }
    };
    let method = request.method().clone();
    if !matches!(method, Method::POST | Method::GET | Method::DELETE) {
        return failure(StatusCode::METHOD_NOT_ALLOWED);
    }
    let session = match input::session(request.headers()) {
        Ok(session) => session,
        Err(status) => {
            state.deny(&owner, "session_header_invalid");
            return failure(status);
        }
    };
    let (binding, mut reservation) = if let Some(id) = session {
        if let Err(status) = input::legacy_headers(request.headers()) {
            state.deny(&owner, "protocol_invalid");
            return failure(status);
        }
        let found = state
            .manager
            .pool
            .find(&id)
            .filter(|binding| binding.owner == owner && binding.credential == credential)
            .map(|binding| (state.validity(&binding), binding));
        let binding = match found {
            Some((Validity::Valid, binding)) => binding,
            Some((Validity::Unavailable, _)) => {
                // 会话是否还有效未知：回 503，客户端稍后带原会话号重试；回 404 会让它丢掉可能
                // 仍有效的会话，回 401 会让它转去重新认证。
                state.deny(&owner, IDENTITY_UNAVAILABLE);
                return failure(StatusCode::SERVICE_UNAVAILABLE);
            }
            _ => {
                // 查不到、已过期或属于别的主体都按会话已结束回 404：客户端据此不带会话号重新
                // initialize；回 401 会被当成令牌失效而转去重新认证（BUG-029）。
                state.deny(&owner, "session_invalid");
                return failure(StatusCode::NOT_FOUND);
            }
        };
        (binding, None)
    } else {
        // 除 initialize 外缺会话号回 400；POST 的首包是不是 initialize 由 input::post 判。
        if method != Method::POST {
            state.deny(&owner, "session_required");
            return failure(StatusCode::BAD_REQUEST);
        }
        let binding = match state
            .manager
            .pool
            .reserve(&owner, &credential, Instant::now())
        {
            Ok(binding) => binding,
            Err(error) => {
                state.deny(&owner, "session_capacity");
                return failure(if error == ReserveError::Full {
                    StatusCode::TOO_MANY_REQUESTS
                } else {
                    StatusCode::SERVICE_UNAVAILABLE
                });
            }
        };
        (
            binding.clone(),
            Some(Reservation::new(binding, state.manager.clone())),
        )
    };
    let initialize = reservation.is_some();
    let work = async {
        let request = if method == Method::POST {
            input::post(request, initialize).await?
        } else {
            request
        };
        Ok::<_, StatusCode>(
            REQUEST_BINDING
                .scope(binding.clone(), next.run(request))
                .await,
        )
    };
    let response = match tokio::time::timeout(state.manager.pool.limits.initialize, work).await {
        Ok(Ok(response)) => response,
        Ok(Err(status)) => {
            state.deny(&owner, "initialize_invalid");
            return failure(status);
        }
        Err(_) => {
            state.manager.discard(binding.clone());
            state.deny(&owner, "request_timeout");
            return failure(StatusCode::REQUEST_TIMEOUT);
        }
    };
    if !response.status().is_success() {
        return response;
    }
    if initialize {
        let validity = state.validity(&binding);
        if validity == Validity::Unavailable {
            state.deny(&owner, IDENTITY_UNAVAILABLE);
        }
        if validity != Validity::Valid
            || response
                .headers()
                .get(input::SESSION)
                .and_then(|value| value.to_str().ok())
                != binding.session.get().map(|id| id.as_ref())
        {
            return failure(StatusCode::SERVICE_UNAVAILABLE);
        }
    }
    // DELETE 已关闭 worker，不能再以 live 检查吞掉正常的关闭响应。
    if method == Method::DELETE {
        return response;
    }
    // 请求已交给 SDK 之后的复核：凭据途中被吊销或轮换回 401；身份库读不出来时凭据真假未知，
    // 回 503 而不是 401；会话途中结束时 SDK 可能已在处理、结果未知，回 503 而不是 404，
    // 免得客户端当成「没处理过」而重新握手后自动重放。
    match binding.current(&state.app.store) {
        Ok(true) => {}
        Ok(false) => return failure(StatusCode::UNAUTHORIZED),
        Err(_) => {
            state.deny(&owner, IDENTITY_UNAVAILABLE);
            return failure(StatusCode::SERVICE_UNAVAILABLE);
        }
    }
    if !binding.live(Instant::now()) {
        return failure(StatusCode::SERVICE_UNAVAILABLE);
    }
    let (parts, body) = response.into_parts();
    let body = body::GuardedBody::wrap(
        body,
        state,
        binding,
        permit,
        reservation.take(),
        method == Method::GET,
    );
    Response::from_parts(parts, body)
}
