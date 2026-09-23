//! 各处理器共用的辅助：取策略平面与主体、在阻塞线程上执行一次操作、时间戳与 JSON 响应。

use std::sync::Arc;

use axum::{
    Json,
    extract::Request,
    response::{IntoResponse, Response},
};
use serde::Serialize;

use super::{PolicyPlane, problem::Problem};
use crate::{
    AppState,
    auth_http::policy::{Input, PolicyPrincipal},
};

pub(super) fn plane(request: &Request) -> Result<Arc<PolicyPlane>, Problem> {
    request
        .extensions()
        .get::<Arc<PolicyPlane>>()
        .cloned()
        .ok_or_else(Problem::absent)
}

pub(super) fn principal(
    state: &AppState,
    request: &mut Request,
) -> Result<(PolicyPrincipal, Option<Input>), Box<Response>> {
    let Some(auth) = &state.sessions else {
        return Err(Box::new(Problem::absent().into_response()));
    };
    auth.policy(request)
}

/// 在阻塞线程上执行一次策略库／账本操作；主体在闭包内重验会话。
pub(super) async fn run<T: Send + 'static>(
    plane: Arc<PolicyPlane>,
    principal: PolicyPrincipal,
    operation: impl FnOnce(&PolicyPlane, &PolicyPrincipal) -> Result<T, Problem> + Send + 'static,
) -> Result<T, Problem> {
    let permit = Arc::clone(&plane.permits)
        .try_acquire_owned()
        .map_err(|_| Problem::busy())?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        principal.verify().map_err(Problem::session)?;
        operation(&plane, &principal)
    })
    .await
    .map_err(|_| Problem::unknown())?
}

pub(super) fn now() -> String {
    jiff::Timestamp::now().to_string()
}

pub(super) fn respond<T: Serialize>(result: Result<T, Problem>) -> Response {
    match result {
        Ok(value) => Json(value).into_response(),
        Err(problem) => problem.into_response(),
    }
}
