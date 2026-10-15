//! L3 发布：发布 desired、回滚版本与解除全局吊销；Intent 先落账，L3 或事务被拒也记审计。

use axum::{
    extract::{Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;
use studiolo_policy::{
    Activation, PolicyError,
    audit::{self, ActivationId, ActivationRequest, Principal},
};

use super::{
    PolicyPlane,
    common::{now, plane, principal, respond, run},
    problem::Problem,
};
use crate::{
    AppState,
    auth_http::policy::{ActivateRequest, Input, L3Denial},
};

#[derive(Serialize)]
#[serde(untagged)]
enum ActivateBody {
    Activated {
        activation: Activation,
        audit_confirmed: bool,
    },
    Released {
        released: bool,
        effective_hash: String,
        audit_confirmed: bool,
    },
}

fn deny(
    plane: &PolicyPlane,
    request: &ActivationRequest<'_>,
    reason: &str,
    problem: Problem,
) -> Problem {
    // 拒绝也进审计；账本写不进时以账本错误为准（失败关闭）。
    match audit::activate_denied(Principal::Owner, request, reason) {
        Ok(event) => match plane.ledger.append(event) {
            Ok(_) => problem,
            Err(error) => Problem::audit(error),
        },
        Err(error) => Problem::audit(error),
    }
}

pub(super) async fn activate(State(state): State<AppState>, mut request: Request) -> Response {
    let plane = match plane(&request) {
        Ok(plane) => plane,
        Err(problem) => return problem.into_response(),
    };
    let (principal, action) = match principal(&state, &mut request) {
        Ok(value) => value,
        Err(response) => return *response,
    };
    let Some(Input::Activate {
        request: wanted,
        expected_effective_hash,
    }) = action
    else {
        return Problem::new(StatusCode::BAD_REQUEST, "invalid", "请求格式无效。").into_response();
    };
    respond(
        run(plane, principal, move |plane, principal| {
            let desired_id = match &wanted {
                ActivateRequest::Desired(id) => Some(id.as_str()),
                _ => None,
            };
            let request = ActivationRequest {
                desired_id,
                rollback_of: match &wanted {
                    ActivateRequest::Rollback(version) => Some(*version),
                    _ => None,
                },
                release_global_revocation: matches!(wanted, ActivateRequest::ReleaseGlobal),
                expected_effective_hash: &expected_effective_hash,
            };
            // Intent 先落账；账本不可用即失败关闭，不发布。
            let id = ActivationId::new().map_err(Problem::audit)?;
            plane
                .ledger
                .append(
                    id.intent(Principal::Owner, &request)
                        .map_err(Problem::audit)?,
                )
                .map_err(Problem::audit)?;
            if let Err(denial) = principal.require_l3() {
                let (reason, problem) = match denial {
                    L3Denial::NoFactorPath => (
                        "no_l3_path",
                        Problem::new(
                            StatusCode::CONFLICT,
                            "no_l3_path",
                            "本入口未启用二次认证，没有 One-time Code 路径，不能发布。",
                        ),
                    ),
                    L3Denial::NoTrustedSource => (
                        "untrusted_source",
                        Problem::new(
                            StatusCode::FORBIDDEN,
                            "untrusted_source",
                            "发布只接受可信 Unix 入口的请求。",
                        ),
                    ),
                    L3Denial::ReauthenticationRequired => (
                        "reauthentication_required",
                        Problem::new(
                            StatusCode::FORBIDDEN,
                            "reauthentication_required",
                            "发布策略需要先完成二次认证（One-time Code）。",
                        ),
                    ),
                    L3Denial::Session(error) => ("session", Problem::session(error)),
                };
                return Err(deny(plane, &request, reason, problem));
            }
            let now = now();
            let outcome = match &wanted {
                ActivateRequest::Desired(desired) => plane
                    .store
                    .activate(desired, &expected_effective_hash, "owner", &now)
                    .map(ActivateBody::from_activation),
                ActivateRequest::Rollback(version) => plane
                    .store
                    .rollback(*version, &expected_effective_hash, "owner", &now)
                    .map(ActivateBody::from_activation),
                ActivateRequest::ReleaseGlobal => {
                    let current = plane
                        .store
                        .effective()
                        .map_err(Problem::policy)?
                        .effective_hash;
                    if current != expected_effective_hash {
                        Err(PolicyError::HashMismatch { current })
                    } else {
                        plane
                            .store
                            .release_global("owner", &now)
                            .map(|()| ActivateBody::Released {
                                released: true,
                                effective_hash: current,
                                audit_confirmed: true,
                            })
                    }
                }
            };
            let body = match outcome {
                Ok(body) => body,
                Err(error) => {
                    let reason = match &error {
                        PolicyError::HashMismatch { .. } => "hash_mismatch",
                        PolicyError::NewerDesiredExists { .. } => "newer_desired_exists",
                        PolicyError::ValidationFailed => "validation_failed",
                        PolicyError::DesiredNotFound | PolicyError::VersionNotFound => "not_found",
                        PolicyError::DesiredNotActivatable(_) => "not_activatable",
                        PolicyError::NotRevoked => "not_revoked",
                        _ => "store_unavailable",
                    };
                    return Err(deny(plane, &request, reason, Problem::policy(error)));
                }
            };
            // Outcome 落账失败：发布已持久，如实标记「审计未确认」而不是假装没发布。
            let confirmed = match &body {
                ActivateBody::Activated { activation, .. } => {
                    let outcome = id
                        .outcome(Principal::Owner, activation)
                        .map_err(Problem::audit)?;
                    let confirmed = plane.ledger.append(outcome).is_ok();
                    if confirmed && activation.cleared_type_revocation {
                        let _ = audit::type_restored(
                            Principal::Owner,
                            &activation.subject,
                            activation.version,
                        )
                        .map(|event| plane.ledger.append(event));
                    }
                    confirmed
                }
                ActivateBody::Released { effective_hash, .. } => {
                    let outcome = id
                        .released_outcome(Principal::Owner, effective_hash)
                        .map_err(Problem::audit)?;
                    let confirmed = plane.ledger.append(outcome).is_ok();
                    if confirmed {
                        let _ = audit::global_released(Principal::Owner)
                            .map(|event| plane.ledger.append(event));
                    }
                    confirmed
                }
            };
            Ok(body.with_confirmation(confirmed))
        })
        .await,
    )
}

impl ActivateBody {
    fn from_activation(activation: Activation) -> Self {
        Self::Activated {
            activation,
            audit_confirmed: true,
        }
    }
    fn with_confirmation(self, confirmed: bool) -> Self {
        match self {
            Self::Activated { activation, .. } => Self::Activated {
                activation,
                audit_confirmed: confirmed,
            },
            Self::Released {
                released,
                effective_hash,
                ..
            } => Self::Released {
                released,
                effective_hash,
                audit_confirmed: confirmed,
            },
        }
    }
}
