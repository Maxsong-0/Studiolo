//! 策略平面人侧入口（M4）：desired → 校验 → diff → L3 发布 → effective；版本与回滚；三级吊销。
//!
//! 全部路由挂在会话门禁之内；策略库句柄由可信启动层经 `with_policy_plane` 装上，
//! 缺它时一律 404（与健康页同一形态）。发布动作固定顺序：重验会话 → 审计 Intent →
//! `require_l3` → 策略库事务 → 审计 Outcome；账本不可用即失败关闭，不发布。

use std::sync::Arc;

use axum::{
    Extension, Json, Router,
    extract::{Path, Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::Serialize;
use studiolo_audit_ledger::{AuditError, AuditLedger};
use studiolo_auth::SessionError;
use studiolo_policy::{
    Activation, DesiredInput, DesiredRecord, DesiredStatus, PolicyError, PolicyStore,
    RevocationLevel, Submitter,
    audit::{self, ActivationId, ActivationRequest, Principal},
};
use tokio::sync::Semaphore;

use crate::{
    AppState,
    auth_http::policy::{self as input, ActivateRequest, Input, L3Denial, PolicyPrincipal},
};

/// 策略平面句柄与它的审计账本；由可信启动层构造一次。
pub struct PolicyPlane {
    store: PolicyStore,
    ledger: Arc<dyn AuditLedger>,
    permits: Arc<Semaphore>,
}

impl PolicyPlane {
    /// 组合主人发布者句柄与人侧权威账本。
    pub fn new(store: PolicyStore, ledger: Arc<dyn AuditLedger>) -> Self {
        Self {
            store,
            ledger,
            permits: Arc::new(Semaphore::new(2)),
        }
    }
}

/// 由可信启动层调用：把已打开的策略平面装到整套路由上（只加一层 `Extension`）。
pub fn with_policy_plane(app: Router, plane: Arc<PolicyPlane>) -> Router {
    app.layer(Extension(plane))
}

/// 装配了会话门禁时挂载策略路由；必须在 `security::guard` 那一层之前调用。
pub(crate) fn routes(routes: Router<AppState>) -> Router<AppState> {
    routes
        .route(input::STATUS, get(status))
        .route(input::DESIRED, get(desired_list).post(desired_submit))
        .route(input::VALIDATE, post(validate))
        .route(input::WITHDRAW, post(withdraw))
        .route(input::ACTIVATE, post(activate))
        .route(input::VERSIONS, get(versions))
        .route("/api/v1/policy/versions/{version}", get(version))
        .route(input::REVOKE, post(revoke))
}

struct Problem {
    status: StatusCode,
    body: serde_json::Value,
}

impl Problem {
    fn new(status: StatusCode, code: &'static str, error: impl Into<String>) -> Self {
        Self {
            status,
            body: serde_json::json!({"code": code, "error": error.into()}),
        }
    }
    fn absent() -> Self {
        Self::new(
            StatusCode::NOT_FOUND,
            "policy_plane_absent",
            "本入口未装配策略平面。",
        )
    }
    fn busy() -> Self {
        Self::new(
            StatusCode::TOO_MANY_REQUESTS,
            "busy",
            "策略操作已达并发限制，请稍后重试。",
        )
    }
    fn unknown() -> Self {
        Self::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "outcome_unknown",
            "结果尚不能确认，请核对当前 effective 后再决定下一步。",
        )
    }
    fn session(error: SessionError) -> Self {
        match error {
            SessionError::Rejected => {
                Self::new(StatusCode::UNAUTHORIZED, "session", "需要有效的应用会话。")
            }
            SessionError::RateLimited => Self::new(
                StatusCode::TOO_MANY_REQUESTS,
                "rate_limited",
                "认证请求已达限制。",
            ),
            _ => Self::unknown(),
        }
    }
    fn policy(error: PolicyError) -> Self {
        match error {
            PolicyError::HashMismatch { current } => Self {
                status: StatusCode::CONFLICT,
                body: serde_json::json!({
                    "code": "hash_mismatch",
                    "error": "effective 已变化，请重新校验后再发布。",
                    "current_effective_hash": current,
                }),
            },
            PolicyError::ValidationFailed => Self::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "validation_failed",
                "校验未通过；逐条问题见该 desired 的报告。",
            ),
            PolicyError::DesiredNotFound | PolicyError::VersionNotFound => {
                Self::new(StatusCode::NOT_FOUND, "not_found", error.to_string())
            }
            PolicyError::DesiredNotActivatable(_)
            | PolicyError::NotRevoked
            | PolicyError::AlreadyBootstrapped => {
                Self::new(StatusCode::CONFLICT, "state", error.to_string())
            }
            PolicyError::InvalidKind
            | PolicyError::InvalidSubject
            | PolicyError::InvalidBody(_)
            | PolicyError::TooManyProposals => {
                Self::new(StatusCode::BAD_REQUEST, "invalid", error.to_string())
            }
            PolicyError::UnsafeStorage
            | PolicyError::MissingStore
            | PolicyError::CorruptStore
            | PolicyError::StoreUnavailable => Self::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "store_unavailable",
                "策略私有库不可用。",
            ),
        }
    }
    fn audit(_: AuditError) -> Self {
        Self::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "audit_unavailable",
            "权威审计不可用：策略动作失败关闭，未发布。",
        )
    }
}

impl IntoResponse for Problem {
    fn into_response(self) -> Response {
        (self.status, Json(self.body)).into_response()
    }
}

fn plane(request: &Request) -> Result<Arc<PolicyPlane>, Problem> {
    request
        .extensions()
        .get::<Arc<PolicyPlane>>()
        .cloned()
        .ok_or_else(Problem::absent)
}

fn principal(
    state: &AppState,
    request: &mut Request,
) -> Result<(PolicyPrincipal, Option<Input>), Box<Response>> {
    let Some(auth) = &state.sessions else {
        return Err(Box::new(Problem::absent().into_response()));
    };
    auth.policy(request)
}

/// 在阻塞线程上执行一次策略库／账本操作；主体在闭包内重验会话。
async fn run<T: Send + 'static>(
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

fn now() -> String {
    jiff::Timestamp::now().to_string()
}

fn respond<T: Serialize>(result: Result<T, Problem>) -> Response {
    match result {
        Ok(value) => Json(value).into_response(),
        Err(problem) => problem.into_response(),
    }
}

#[derive(Serialize)]
struct StatusBody {
    effective_hash: String,
    entries: Vec<studiolo_policy::EffectiveEntry>,
    global_revoked: bool,
    pending_desired: usize,
    revocations: Vec<studiolo_policy::RevocationRecord>,
    l3_available: bool,
}

async fn status(State(state): State<AppState>, mut request: Request) -> Response {
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

async fn desired_list(State(state): State<AppState>, mut request: Request) -> Response {
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
struct DesiredBody {
    desired: DesiredRecord,
}

async fn desired_submit(State(state): State<AppState>, mut request: Request) -> Response {
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

async fn validate(State(state): State<AppState>, mut request: Request) -> Response {
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

async fn withdraw(State(state): State<AppState>, mut request: Request) -> Response {
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

async fn activate(State(state): State<AppState>, mut request: Request) -> Response {
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

#[derive(Serialize)]
struct VersionsBody {
    versions: Vec<studiolo_policy::VersionMeta>,
}

async fn versions(State(state): State<AppState>, mut request: Request) -> Response {
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

async fn version(
    State(state): State<AppState>,
    Path(number): Path<u64>,
    mut request: Request,
) -> Response {
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

#[derive(Serialize)]
struct RevokeResponse {
    created: bool,
    level: RevocationLevel,
    target: String,
}

async fn revoke(State(state): State<AppState>, mut request: Request) -> Response {
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

// 让 `DesiredStatus` 等类型在响应里保持库内稳定名，前端按同一枚举解析。
#[allow(dead_code)]
fn _status_names(status: DesiredStatus) -> &'static str {
    status.as_str()
}
