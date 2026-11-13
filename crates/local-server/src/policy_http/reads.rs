//! 只读审计查询；与策略动作同一认证、语法门禁和有界阻塞调度。
use super::{
    common::{plane, principal, respond, run},
    problem::Problem,
};
use crate::{AppState, auth_http::policy::Input};
use axum::{
    extract::{Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
pub(super) async fn query(State(state): State<AppState>, mut request: Request) -> Response {
    let plane = match plane(&request) {
        Ok(v) => v,
        Err(p) => return p.into_response(),
    };
    let (principal, action) = match principal(&state, &mut request) {
        Ok(v) => v,
        Err(r) => return *r,
    };
    let Some(Input::Reads(query)) = action else {
        return Problem::new(StatusCode::BAD_REQUEST, "invalid", "请求格式无效。").into_response();
    };
    respond(
        run(plane, principal, move |plane, _| {
            let audit = plane.read_audit.as_ref().ok_or_else(|| {
                Problem::new(
                    StatusCode::NOT_FOUND,
                    "reads_plane_absent",
                    "本入口未装配接入方读取审计。",
                )
            })?;
            audit.query(&query).map_err(|_| {
                Problem::new(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "reads_unavailable",
                    "审计账本暂不可用或完整性验证未通过，请稍后重试。",
                )
            })
        })
        .await,
    )
}
