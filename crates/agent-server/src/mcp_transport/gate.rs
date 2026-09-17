//! 所有 HTTP 方法先匹配预占中的凭据与 SID，再交给 rmcp。
use super::{
    TransportState, body, input,
    manager::{REQUEST_BINDING, Reservation},
    pool::ReserveError,
};
use crate::auth::{AgentType, CredentialDigest};
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
        let binding = state.manager.pool.find(&id).filter(|binding| {
            binding.owner == owner && binding.credential == credential && state.valid(binding)
        });
        let Some(binding) = binding else {
            state.deny(&owner, "session_invalid");
            return failure(StatusCode::UNAUTHORIZED);
        };
        (binding, None)
    } else {
        if method != Method::POST {
            state.deny(&owner, "session_required");
            return failure(StatusCode::UNAUTHORIZED);
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
    if initialize
        && (!state.valid(&binding)
            || response
                .headers()
                .get(input::SESSION)
                .and_then(|value| value.to_str().ok())
                != binding.session.get().map(|id| id.as_ref()))
    {
        return failure(StatusCode::SERVICE_UNAVAILABLE);
    }
    // DELETE 已关闭 worker，不能再以 live 检查吞掉正常的关闭响应。
    if method == Method::DELETE {
        return response;
    }
    if !state.valid(&binding) {
        return failure(StatusCode::UNAUTHORIZED);
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
