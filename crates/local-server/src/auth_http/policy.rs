//! 策略平面端点的输入解析与主体提取：L2 读写与 L3 发布共用同一条会话链。
//!
//! 只在这里出现路径常量与方法表；`gate`、`security`、`input` 与 `policy_http` 都从这里取，
//! 新增端点漏改一侧即门禁旁路的老毛病不再有第二份表可漏（T-026 复审 #22 的策略端点部分）。

use std::{net::IpAddr, sync::Arc};

use axum::{extract::Request, response::Response};
use serde::Deserialize;
use studiolo_auth::{
    AccessIdentity, FactorRuntime, SensitiveAction, SessionError, SessionInfo, SessionManager,
    TotpRequest,
};
use studiolo_policy::{PolicyKind, RevocationLevel};

use super::{AuthHttp, Context, cookie::Credentials, input::Action, responses::Failure};

/// effective 摘要与生效中的吊销。
pub(crate) const STATUS: &str = "/api/v1/policy";
/// GET 列 desired；POST 提交 desired。
pub(crate) const DESIRED: &str = "/api/v1/policy/desired";
/// 复核一份 desired。
pub(crate) const VALIDATE: &str = "/api/v1/policy/validate";
/// 撤回一份 desired。
pub(crate) const WITHDRAW: &str = "/api/v1/policy/withdraw";
/// 发布／回滚／解除全局吊销（L3）。
pub(crate) const ACTIVATE: &str = "/api/v1/policy/activate";
/// 版本列表；`/api/v1/policy/versions/{n}` 读某版正文。
pub(crate) const VERSIONS: &str = "/api/v1/policy/versions";
/// 三级吊销（L2）。
pub(crate) const REVOKE: &str = "/api/v1/agents/revoke";
/// 单份 desired 的 JSON 正文上限（规则母本 256 KiB ＋ 转义余量）。
pub(crate) const JSON_LIMIT: usize = 512 * 1024;

/// 策略端点的方法表：单一来源。
pub(crate) fn method_for(path: &str) -> Option<&'static str> {
    match path {
        STATUS | VERSIONS => Some("GET"),
        DESIRED => Some("GET, POST"),
        VALIDATE | WITHDRAW | ACTIVATE | REVOKE => Some("POST"),
        _ if path
            .strip_prefix("/api/v1/policy/versions/")
            .is_some_and(|rest| {
                !rest.is_empty() && rest.bytes().all(|byte| byte.is_ascii_digit())
            }) =>
        {
            Some("GET")
        }
        _ => None,
    }
}

pub(crate) fn is_policy_path(path: &str) -> bool {
    method_for(path).is_some()
}

pub(crate) fn is_policy_post(path: &str) -> bool {
    matches!(path, DESIRED | VALIDATE | WITHDRAW | ACTIVATE | REVOKE)
}

/// 只有发布需要 L3，因而只有它需要可信 Unix 来源证明。
pub(crate) fn requires_l3(path: &str) -> bool {
    path == ACTIVATE
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DesiredBody {
    kind: String,
    subject: String,
    /// 必须显式出现：字符串＝正文，`null`＝撤销；缺省即拒绝，漏写字段不能变成撤销请求。
    #[serde(default, deserialize_with = "explicit")]
    body: Option<Option<String>>,
}

fn explicit<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Option<String>>, D::Error> {
    Option::<String>::deserialize(deserializer).map(Some)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DesiredRef {
    desired_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ActivateBody {
    #[serde(default)]
    desired_id: Option<String>,
    #[serde(default)]
    rollback_to: Option<u64>,
    #[serde(default)]
    release_global_revocation: Option<bool>,
    expected_effective_hash: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RevokeBody {
    level: String,
    #[serde(default)]
    target: Option<String>,
    reason: String,
}

/// 发布请求的三种形态之一。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ActivateRequest {
    /// 发布一份 desired。
    Desired(String),
    /// 回滚到某个版本。
    Rollback(u64),
    /// 解除全局吊销。
    ReleaseGlobal,
}

/// 已解析的策略端点输入。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Input {
    Submit {
        kind: PolicyKind,
        subject: String,
        body: Option<String>,
    },
    Validate {
        desired_id: String,
    },
    Withdraw {
        desired_id: String,
    },
    Activate {
        request: ActivateRequest,
        expected_effective_hash: String,
    },
    Revoke {
        level: RevocationLevel,
        target: String,
        reason: String,
    },
}

fn valid_id(value: &str) -> bool {
    value.len() == 26 && value.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

pub(super) fn parse(path: &str, bytes: &[u8]) -> Result<Input, Failure> {
    let bad = |_| Failure::bad_request();
    match path {
        DESIRED => {
            let body: DesiredBody = serde_json::from_slice(bytes).map_err(bad)?;
            let text = body.body.ok_or_else(Failure::bad_request)?;
            Ok(Input::Submit {
                kind: PolicyKind::parse(&body.kind).map_err(|_| Failure::bad_request())?,
                subject: body.subject,
                body: text,
            })
        }
        VALIDATE | WITHDRAW => {
            let body: DesiredRef = serde_json::from_slice(bytes).map_err(bad)?;
            if !valid_id(&body.desired_id) {
                return Err(Failure::bad_request());
            }
            Ok(if path == VALIDATE {
                Input::Validate {
                    desired_id: body.desired_id,
                }
            } else {
                Input::Withdraw {
                    desired_id: body.desired_id,
                }
            })
        }
        ACTIVATE => {
            let body: ActivateBody = serde_json::from_slice(bytes).map_err(bad)?;
            if body.expected_effective_hash.len() != 64
                || !body
                    .expected_effective_hash
                    .bytes()
                    .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
            {
                return Err(Failure::bad_request());
            }
            let request = match (
                body.desired_id,
                body.rollback_to,
                body.release_global_revocation,
            ) {
                (Some(id), None, None) if valid_id(&id) => ActivateRequest::Desired(id),
                (None, Some(version), None) if version > 0 => ActivateRequest::Rollback(version),
                (None, None, Some(true)) => ActivateRequest::ReleaseGlobal,
                _ => return Err(Failure::bad_request()),
            };
            Ok(Input::Activate {
                request,
                expected_effective_hash: body.expected_effective_hash,
            })
        }
        REVOKE => {
            let body: RevokeBody = serde_json::from_slice(bytes).map_err(bad)?;
            let level = RevocationLevel::parse(&body.level).ok_or_else(Failure::bad_request)?;
            let target = match (level, body.target) {
                (RevocationLevel::Global, None) => "*".to_owned(),
                (RevocationLevel::Global, Some(_)) => return Err(Failure::bad_request()),
                (_, Some(target)) if !target.is_empty() && target.len() <= 64 => target,
                _ => return Err(Failure::bad_request()),
            };
            if body.reason.trim().is_empty()
                || body.reason.chars().count() > 200
                || body.reason.chars().any(char::is_control)
            {
                return Err(Failure::bad_request());
            }
            Ok(Input::Revoke {
                level,
                target,
                reason: body.reason,
            })
        }
        _ => Err(Failure::bad_request()),
    }
}

/// 策略端点的主体：每次动作前重验会话；发布前再验 L3。不缓存任何授权结果。
pub(crate) struct PolicyPrincipal {
    sessions: Arc<SessionManager>,
    factors: Option<Arc<FactorRuntime>>,
    identity: AccessIdentity,
    credentials: Arc<Credentials>,
    source: Option<IpAddr>,
}

/// 发布前 L3 检查的结构化结果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum L3Denial {
    /// 本入口没有因子运行时（非因子模式）：没有 L3 路径。
    NoFactorPath,
    /// 没有可信 Unix 来源证明。
    NoTrustedSource,
    /// 需要重新完成二次认证。
    ReauthenticationRequired,
    /// 会话或因子检查失败（会话失效、限流、库不可用）。
    Session(SessionError),
}

impl PolicyPrincipal {
    pub(crate) fn verify(&self) -> Result<SessionInfo, SessionError> {
        self.sessions.check_with_csrf(
            &self.identity,
            &self.credentials.token,
            &self.credentials.csrf,
        )
    }

    /// 本入口是否具备 L3 路径（因子模式）。
    pub(crate) fn l3_available(&self) -> bool {
        self.factors.is_some()
    }

    /// 发布前即时检查：重验 Access／会话／CSRF／活动因子与未过期的近期认证。
    pub(crate) fn require_l3(&self) -> Result<(), L3Denial> {
        let Some(factors) = &self.factors else {
            return Err(L3Denial::NoFactorPath);
        };
        let Some(source) = self.source else {
            return Err(L3Denial::NoTrustedSource);
        };
        let request = TotpRequest {
            identity: &self.identity,
            token: &self.credentials.token,
            csrf: &self.credentials.csrf,
            source,
        };
        match factors.require_l3(&request, SensitiveAction::ActivatePolicy) {
            Ok(()) => Ok(()),
            Err(SessionError::ReauthenticationRequired) => Err(L3Denial::ReauthenticationRequired),
            Err(error) => Err(L3Denial::Session(error)),
        }
    }
}

impl AuthHttp {
    /// 取出策略端点的主体与已解析输入（GET 没有输入）。
    pub(crate) fn policy(
        &self,
        request: &mut Request,
    ) -> Result<(PolicyPrincipal, Option<Input>), Box<Response>> {
        let context = request
            .extensions_mut()
            .remove::<Arc<Context>>()
            .ok_or_else(|| Failure::unauthorized().response())?;
        let context = Arc::try_unwrap(context).map_err(|_| Failure::unavailable().response())?;
        let input = match context.action {
            None => None,
            Some(Action::Policy(input)) => Some(input),
            Some(_) => return Err(Box::new(Failure::bad_request().response())),
        };
        Ok((
            PolicyPrincipal {
                sessions: self.sessions.clone(),
                factors: self.factors.clone(),
                identity: context.identity,
                credentials: context
                    .credentials
                    .ok_or_else(|| Failure::unauthorized().response())?,
                source: context.source.as_ref().map(|source| source.ip()),
            },
            input,
        ))
    }
}
