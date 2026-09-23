//! L2 写动作：提交、复核、撤回 desired 与三级吊销；都不需要 L3，主体照样逐次重验会话。

use axum::{
    extract::{Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;
use studiolo_policy::{
    DesiredInput, DesiredRecord, RevocationLevel, Submitter,
    audit::{self, Principal},
};

use super::{
    common::{now, plane, principal, respond, run},
    problem::Problem,
};
use crate::{AppState, auth_http::policy::Input};

#[derive(Serialize)]
struct DesiredBody {
    desired: DesiredRecord,
}

pub(super) async fn desired_submit(
    State(state): State<AppState>,
    mut request: Request,
) -> Response {
    let plane = match plane(&request) {
        Ok(plane) => plane,
        Err(problem) => return problem.into_response(),
    };
    let (principal, action) = match principal(&state, &mut request) {
        Ok(value) => value,
        Err(response) => return *response,
    };
    let Some(Input::Submit {
        kind,
        subject,
        body,
    }) = action
    else {
        return Problem::new(StatusCode::BAD_REQUEST, "invalid", "请求格式无效。").into_response();
    };
    respond(
        run(plane, principal, move |plane, _| {
            let record = plane
                .store
                .submit_desired(
                    DesiredInput {
                        kind,
                        subject,
                        body,
                    },
                    Submitter::Owner,
                    &now(),
                )
                .map_err(Problem::policy)?;
            plane
                .ledger
                .append(
                    audit::desired_submitted(Principal::Owner, &record).map_err(Problem::audit)?,
                )
                .map_err(Problem::audit)?;
            Ok(DesiredBody { desired: record })
        })
        .await,
    )
}

pub(super) async fn validate(State(state): State<AppState>, mut request: Request) -> Response {
    let plane = match plane(&request) {
        Ok(plane) => plane,
        Err(problem) => return problem.into_response(),
    };
    let (principal, action) = match principal(&state, &mut request) {
        Ok(value) => value,
        Err(response) => return *response,
    };
    let Some(Input::Validate { desired_id }) = action else {
        return Problem::new(StatusCode::BAD_REQUEST, "invalid", "请求格式无效。").into_response();
    };
    respond(
        run(plane, principal, move |plane, _| {
            let record = plane
                .store
                .validate_desired(&desired_id, &now())
                .map_err(Problem::policy)?;
            plane
                .ledger
                .append(audit::validated(Principal::Owner, &record).map_err(Problem::audit)?)
                .map_err(Problem::audit)?;
            Ok(DesiredBody { desired: record })
        })
        .await,
    )
}

pub(super) async fn withdraw(State(state): State<AppState>, mut request: Request) -> Response {
    let plane = match plane(&request) {
        Ok(plane) => plane,
        Err(problem) => return problem.into_response(),
    };
    let (principal, action) = match principal(&state, &mut request) {
        Ok(value) => value,
        Err(response) => return *response,
    };
    let Some(Input::Withdraw { desired_id }) = action else {
        return Problem::new(StatusCode::BAD_REQUEST, "invalid", "请求格式无效。").into_response();
    };
    respond(
        run(plane, principal, move |plane, _| {
            let record = plane
                .store
                .withdraw_desired(&desired_id, &now())
                .map_err(Problem::policy)?;
            Ok(DesiredBody { desired: record })
        })
        .await,
    )
}

#[derive(Serialize)]
struct RevokeResponse {
    created: bool,
    level: RevocationLevel,
    target: String,
}

pub(super) async fn revoke(State(state): State<AppState>, mut request: Request) -> Response {
    let plane = match plane(&request) {
        Ok(plane) => plane,
        Err(problem) => return problem.into_response(),
    };
    let (principal, action) = match principal(&state, &mut request) {
        Ok(value) => value,
        Err(response) => return *response,
    };
    let Some(Input::Revoke {
        level,
        target,
        reason,
    }) = action
    else {
        return Problem::new(StatusCode::BAD_REQUEST, "invalid", "请求格式无效。").into_response();
    };
    respond(
        run(plane, principal, move |plane, _| {
            let created = plane
                .store
                .revoke(level, &target, "owner", &reason, &now())
                .map_err(Problem::policy)?;
            if created {
                // 安全方向：吊销先生效，审计随后补记；账本不可用不撤回吊销。
                if let Ok(event) = audit::revoked(Principal::Owner, level, &target, &reason) {
                    let _ = plane.ledger.append(event);
                }
            }
            Ok(RevokeResponse {
                created,
                level,
                target: if level == RevocationLevel::Global {
                    "*".into()
                } else {
                    target
                },
            })
        })
        .await,
    )
}
