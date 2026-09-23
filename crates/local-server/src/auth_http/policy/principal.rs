//! 策略端点的主体：从本次请求取出会话凭据与已解析输入；动作前重验会话，发布前再验 L3。

use std::{net::IpAddr, sync::Arc};

use axum::{extract::Request, response::Response};
use studiolo_auth::{
    AccessIdentity, FactorRuntime, SensitiveAction, SessionError, SessionInfo, SessionManager,
    TotpRequest,
};

use super::request::Input;
use crate::auth_http::{AuthHttp, Context, cookie::Credentials, input::Action, responses::Failure};

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
