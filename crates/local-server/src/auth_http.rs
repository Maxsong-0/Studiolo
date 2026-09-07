//! 封闭的应用会话 HTTP Module；不创建存储、不装配因子秘密或内容写权限。

mod cookie;
mod factor_handlers;
mod factor_input;
mod factor_state;
mod gate;
mod handlers;
mod input;
mod responses;

use std::sync::Arc;

use axum::{
    Router,
    extract::Request,
    response::Response,
    routing::{get, post},
};
use studiolo_auth::{AccessIdentity, FactorRuntime, SessionError, SessionInfo, SessionManager};
use tokio::sync::Semaphore;

use crate::AppState;
use cookie::Credentials;
use responses::Failure;

pub(crate) const SELF: &str = "/api/v1/session";
pub(crate) const SESSIONS: &str = "/api/v1/sessions";
pub(crate) const REVOKE: &str = "/api/v1/sessions/revoke";
pub(crate) const STEP_UP: &str = "/api/v1/auth/step-up";
pub(crate) const RECOVERY_CODES: &str = "/api/v1/auth/recovery-codes";
pub(crate) const RECOVER: &str = "/api/v1/auth/factor/recover";
pub(crate) const FACTOR_STATE: &str = "/api/v1/auth/state";

pub(crate) fn is_factor_path(path: &str) -> bool {
    matches!(path, STEP_UP | RECOVERY_CODES | RECOVER)
}

/// 共享许可随阻塞任务存活；HTTP 取消不会把仍运行的库调用从预算中移除。
#[derive(Clone)]
pub(crate) struct AuthHttp {
    sessions: Arc<SessionManager>,
    factors: Option<Arc<FactorRuntime>>,
    permits: Arc<Semaphore>,
}

/// 仅活在本次请求内，不能向其他请求传递成可缓存的授权证明。
struct Context {
    identity: AccessIdentity,
    credentials: Option<Arc<Credentials>>,
    session: Option<SessionInfo>,
    action: Option<input::Action>,
    source: Option<crate::unix_ingress::TrustedNetworkSource>,
}

pub(crate) fn routes(factors: bool) -> Router<AppState> {
    let routes = Router::new()
        .route(SELF, get(handlers::current))
        .route(FACTOR_STATE, get(factor_state::handle))
        .route(SESSIONS, get(handlers::list).post(handlers::issue))
        .route(REVOKE, post(handlers::revoke));
    if factors {
        routes
            .route(STEP_UP, post(factor_handlers::handle))
            .route(RECOVERY_CODES, post(factor_handlers::handle))
            .route(RECOVER, post(factor_handlers::handle))
    } else {
        routes
    }
}

impl AuthHttp {
    pub(crate) fn new(sessions: Arc<SessionManager>) -> Self {
        Self {
            sessions,
            factors: None,
            permits: Arc::new(Semaphore::new(2)),
        }
    }

    /// 因子运行能力由可信启动层装配；HTTP不取得种子或首次登记句柄。
    pub(crate) fn with_factors(sessions: Arc<SessionManager>, factors: Arc<FactorRuntime>) -> Self {
        Self {
            factors: Some(factors),
            ..Self::new(sessions)
        }
    }

    pub(crate) fn has_factors(&self) -> bool {
        self.factors.is_some()
    }

    /// 共通入口已经验证 Host 和本次真实 Access；这里补上 API 的严格同源与会话链。
    pub(crate) async fn authorize(
        &self,
        request: &mut Request,
        identity: AccessIdentity,
        origin: &str,
    ) -> Result<(), Response> {
        self.authorize_inner(request, identity, origin)
            .await
            .map_err(Failure::response)
    }

    async fn authorize_inner(
        &self,
        request: &mut Request,
        identity: AccessIdentity,
        origin: &str,
    ) -> Result<(), Failure> {
        gate::headers(request, origin, self.has_factors())?;
        // Access 已由外层逐请求验证；固定模式不需要应用 Cookie，也不授予内容权限。
        if request.uri().path() == crate::bootstrap::PATH {
            return Ok(());
        }
        let source = if self.has_factors() && is_factor_path(request.uri().path()) {
            Some(crate::unix_ingress::source(request).map_err(|_| Failure::forbidden())?)
        } else {
            None
        };
        for name in ["cookie", "x-csrf-token"] {
            if let Some(value) = request.headers_mut().get_mut(name) {
                value.set_sensitive(true);
            }
        }
        let issuing =
            request.method() == axum::http::Method::POST && request.uri().path() == SESSIONS;
        let value = cookie::value(request.headers())?;
        if issuing && value.is_some() {
            return Err(Failure::conflict());
        }
        let credentials = if issuing {
            None
        } else {
            Some(Arc::new(cookie::parse(value)?))
        };
        if request.method() == axum::http::Method::POST && !issuing {
            gate::csrf(
                request.headers(),
                credentials.as_deref().ok_or_else(Failure::unauthorized)?,
            )?;
        }
        // 解析体先于任何 Auth 调用；未知字段、超限和慢体均不能触发签发/吊销。
        let action = input::action(request).await?;
        let session = if let Some(credentials) = &credentials {
            let credentials = Arc::clone(credentials);
            let identity = identity.clone();
            Some(
                self.run(move |sessions| {
                    sessions.check_with_csrf(&identity, &credentials.token, &credentials.csrf)
                })
                .await?,
            )
        } else {
            None
        };
        request.extensions_mut().insert(Arc::new(Context {
            identity,
            credentials,
            session,
            action,
            source,
        }));
        Ok(())
    }

    async fn run<T: Send + 'static>(
        &self,
        operation: impl FnOnce(&SessionManager) -> Result<T, SessionError> + Send + 'static,
    ) -> Result<T, Failure> {
        let sessions = Arc::clone(&self.sessions);
        self.blocking(move || operation(&sessions))
            .await
            .map_err(Failure::session)
    }

    async fn run_factor<T: Send + 'static>(
        &self,
        operation: impl FnOnce(&FactorRuntime) -> Result<T, SessionError> + Send + 'static,
    ) -> Result<T, Failure> {
        let factors = Arc::clone(self.factors.as_ref().ok_or_else(Failure::unavailable)?);
        self.blocking(move || operation(&factors))
            .await
            .map_err(Failure::factor)
    }

    async fn blocking<T: Send + 'static>(
        &self,
        operation: impl FnOnce() -> Result<T, SessionError> + Send + 'static,
    ) -> Result<T, SessionError> {
        let permit = Arc::clone(&self.permits)
            .try_acquire_owned()
            .map_err(|_| SessionError::StorageUnavailable)?;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            operation()
        })
        .await
        .map_err(|_| SessionError::StorageUnavailable)?
    }
}
