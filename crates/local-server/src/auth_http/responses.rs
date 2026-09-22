//! 固定传输错误和公开投影；凭证只在明确交付点出现，错误不复制底层细节。

use axum::{
    Json,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::Serialize;
use studiolo_auth::{SessionError, SessionInfo, SessionState};

use crate::ApiError;

/// 写操作执行中出错时的措辞：结果可能已经落库，只能请人先核对。
const UNCONFIRMED: &str = "认证结果暂不能确认；操作可能已生效，请先核对状态。";

pub(super) struct Failure {
    status: StatusCode,
    message: &'static str,
    clear_cookie: bool,
    allow: Option<&'static str>,
    code: Option<&'static str>,
}

impl Failure {
    pub fn new(status: StatusCode, message: &'static str) -> Self {
        Self {
            status,
            message,
            clear_cookie: false,
            allow: None,
            code: None,
        }
    }
    pub fn bad_request() -> Self {
        Self::new(StatusCode::BAD_REQUEST, "认证请求格式无效。")
    }
    pub fn forbidden() -> Self {
        Self::new(StatusCode::FORBIDDEN, "认证请求不符合安全策略。")
    }
    pub fn unauthorized() -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            message: "需要有效的应用会话。",
            clear_cookie: true,
            allow: None,
            code: None,
        }
    }
    pub fn unavailable() -> Self {
        Self::new(StatusCode::SERVICE_UNAVAILABLE, UNCONFIRMED)
    }
    /// 许可排队超时：请求根本没有进入认证存储，一定没有生效。
    pub fn busy() -> Self {
        Self::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "认证服务正忙，本次请求没有执行；请稍后重试。",
        )
        .with_code("auth_busy")
    }
    /// 读请求没有副作用：把「写操作可能已生效」的措辞换成读取的实情，其余失败原样保留。
    pub fn for_read(self) -> Self {
        if self.status == StatusCode::SERVICE_UNAVAILABLE && self.message == UNCONFIRMED {
            Self::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "暂时无法确认登录状态；本次读取没有改动任何内容，请稍后重试。",
            )
        } else {
            self
        }
    }
    pub fn conflict() -> Self {
        Self::new(StatusCode::CONFLICT, "已有应用 Cookie，不能隐式换发会话。")
    }
    pub fn method(allow: &'static str) -> Self {
        let mut failure = Self::new(StatusCode::METHOD_NOT_ALLOWED, "此认证入口不接受该方法。");
        failure.allow = Some(allow);
        failure
    }
    pub fn session(error: SessionError) -> Self {
        match error {
            SessionError::Rejected => Self::unauthorized(),
            SessionError::RateLimited => {
                Self::new(StatusCode::TOO_MANY_REQUESTS, "认证请求已达限制。")
            }
            SessionError::FactorUnavailable => {
                Self::new(StatusCode::CONFLICT, "尚未完成可信因子登记。")
            }
            SessionError::OperationPending => {
                Self::new(StatusCode::CONFLICT, "已有认证操作正在处理。")
            }
            _ => Self::unavailable(),
        }
    }
    /// 因子验证失败不代表预检过的Cookie无效；执行期间撤权由后续会话检查确认。
    pub fn factor(error: SessionError) -> Self {
        match error {
            SessionError::Rejected => Self::new(
                StatusCode::FORBIDDEN,
                "因子验证未通过；操作可能已消费验证码，请先核对状态。",
            ),
            SessionError::ReauthenticationRequired => {
                Self::new(StatusCode::FORBIDDEN, "当前操作需要重新完成二次认证。")
            }
            SessionError::FactorUnavailable => {
                Self::new(StatusCode::CONFLICT, "当前没有可用的预配置恢复候选。")
            }
            SessionError::OperationPending => {
                Self::new(StatusCode::CONFLICT, "已有认证操作正在处理；请先核对状态。")
            }
            SessionError::RateLimited => {
                Self::new(StatusCode::TOO_MANY_REQUESTS, "认证请求已达限制。")
            }
            _ => Self::unavailable(),
        }
    }
    pub fn with_code(mut self, code: &'static str) -> Self {
        self.code = Some(code);
        self
    }
    pub fn response(self) -> Response {
        let mut response = if let Some(code) = self.code {
            (
                self.status,
                Json(serde_json::json!({"code":code,"error":self.message})),
            )
                .into_response()
        } else {
            ApiError(self.status, self.message.into()).into_response()
        };
        if let Some(allow) = self.allow {
            response
                .headers_mut()
                .insert(header::ALLOW, axum::http::HeaderValue::from_static(allow));
        }
        if self.clear_cookie {
            clear_cookie(&mut response);
        }
        response
    }
}

pub(super) fn clear_cookie(response: &mut Response) {
    response
        .headers_mut()
        .insert(header::SET_COOKIE, super::cookie::expired());
}

#[derive(Serialize)]
pub(super) struct PublicSession<'a> {
    id: &'a str,
    state: &'static str,
    created_at_ms: i64,
    last_seen_at_ms: i64,
    absolute_expires_at_ms: i64,
}

impl<'a> From<&'a SessionInfo> for PublicSession<'a> {
    fn from(info: &'a SessionInfo) -> Self {
        Self {
            id: info.id.as_str(),
            state: match info.state {
                SessionState::Pending => "pending",
                SessionState::Active => "active",
                SessionState::Revoked => "revoked",
                SessionState::Expired => "expired",
                SessionState::Abandoned => "abandoned",
            },
            created_at_ms: info.created_at_ms,
            last_seen_at_ms: info.last_seen_at_ms,
            absolute_expires_at_ms: info.absolute_expires_at_ms,
        }
    }
}

pub(super) fn owner_scope(identity: &studiolo_auth::AccessIdentity) -> String {
    let bytes = serde_json::to_vec(&(
        "studiolo-owner-scope-v1",
        identity.issuer(),
        identity.audience(),
        identity.subject(),
        identity.email(),
    ))
    .expect("身份字符串可以编码");
    crate::editing::digest(&bytes)
}

pub(super) fn session(
    info: &SessionInfo,
    csrf: &str,
    status: StatusCode,
    owner_scope: &str,
) -> Response {
    #[derive(Serialize)]
    struct Body<'a> {
        session: PublicSession<'a>,
        csrf_token: &'a str,
        owner_scope: &'a str,
    }
    (
        status,
        Json(Body {
            session: info.into(),
            csrf_token: csrf,
            owner_scope,
        }),
    )
        .into_response()
}
