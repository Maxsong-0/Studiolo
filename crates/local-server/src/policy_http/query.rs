//! 只读查询：effective 摘要、desired 列表、版本列表与单版正文；每次仍重验会话。

use axum::{
    extract::{Path, Request, State, rejection::PathRejection},
    response::{IntoResponse, Response},
};
use serde::Serialize;
use studiolo_policy::{DesiredRecord, PolicyError};

use super::{
    common::{plane, principal, respond, run},
    problem::Problem,
};
use crate::AppState;

#[derive(Serialize)]
struct StatusBody {
    effective_hash: String,
    entries: Vec<studiolo_policy::EffectiveEntry>,
    global_revoked: bool,
    pending_desired: usize,
    revocations: Vec<studiolo_policy::RevocationRecord>,
    l3_available: bool,
}

pub(super) async fn status(State(state): State<AppState>, mut request: Request) -> Response {
    let plane = match plane(&request) {
        Ok(plane) => plane,
        Err(problem) => return problem.into_response(),
    };
    let (principal, _) = match principal(&state, &mut request) {
        Ok(value) => value,
        Err(response) => return *response,
    };
    respond(
        run(plane, principal, |plane, principal| {
            let summary = plane.store.effective().map_err(Problem::policy)?;
            Ok(StatusBody {
                effective_hash: summary.effective_hash,
                entries: summary.entries,
                global_revoked: summary.global_revoked,
                pending_desired: summary.pending_desired,
                revocations: plane.store.revocations(true).map_err(Problem::policy)?,
                l3_available: principal.l3_available(),
            })
        })
        .await,
    )
}

#[derive(Serialize)]
struct DesiredList {
    desired: Vec<DesiredRecord>,
}

pub(super) async fn desired_list(State(state): State<AppState>, mut request: Request) -> Response {
    let plane = match plane(&request) {
        Ok(plane) => plane,
        Err(problem) => return problem.into_response(),
    };
    let (principal, _) = match principal(&state, &mut request) {
        Ok(value) => value,
        Err(response) => return *response,
    };
    respond(
        run(plane, principal, |plane, _| {
            Ok(DesiredList {
                desired: plane
                    .store
                    .desired_list(None, 200)
                    .map_err(Problem::policy)?,
            })
        })
        .await,
    )
}

#[derive(Serialize)]
struct VersionsBody {
    versions: Vec<studiolo_policy::VersionMeta>,
}

pub(super) async fn versions(State(state): State<AppState>, mut request: Request) -> Response {
    let plane = match plane(&request) {
        Ok(plane) => plane,
        Err(problem) => return problem.into_response(),
    };
    let (principal, _) = match principal(&state, &mut request) {
        Ok(value) => value,
        Err(response) => return *response,
    };
    respond(
        run(plane, principal, |plane, _| {
            Ok(VersionsBody {
                versions: plane
                    .store
                    .versions(None, None, 200)
                    .map_err(Problem::policy)?,
            })
        })
        .await,
    )
}

#[derive(Serialize)]
struct VersionBody {
    version: studiolo_policy::VersionRecord,
}

pub(super) async fn version(
    State(state): State<AppState>,
    number: Result<Path<u64>, PathRejection>,
    mut request: Request,
) -> Response {
    // 版本号拒绝仍先于平面与主体检查，状态码沿用原提取器；axum 默认拒绝是英文纯文本，
    // 还会回显原始路径段，这里一律换成固定中文 JSON。
    let Path(number) = match number {
        Ok(value) => value,
        Err(rejection) => {
            return Problem::new(
                rejection.status(),
                "invalid_policy_version",
                "策略版本号不合法。",
            )
            .into_response();
        }
    };
    let plane = match plane(&request) {
        Ok(plane) => plane,
        Err(problem) => return problem.into_response(),
    };
    let (principal, _) = match principal(&state, &mut request) {
        Ok(value) => value,
        Err(response) => return *response,
    };
    respond(
        run(plane, principal, move |plane, _| {
            plane
                .store
                .version(number)
                .map_err(Problem::policy)?
                .map(|version| VersionBody { version })
                .ok_or_else(|| Problem::policy(PolicyError::VersionNotFound))
        })
        .await,
    )
}
