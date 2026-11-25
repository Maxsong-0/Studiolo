//! 连接器的答复：全部是 JSON，带 `Studiolo-Connect: 1` 和 `no-store`。
//!
//! 没有这个头的答复不是 Studiolo 回的（边缘质询、隧道或网络），客户端据此分得清是谁说的「不」。
use axum::{
    Json,
    body::Body,
    http::{HeaderName, HeaderValue, Request, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};

/// 每个答复都带的头，值恒为 `1`。
pub(super) const MARKER: HeaderName = HeaderName::from_static("studiolo-connect");

/// 一次拒绝：状态码、稳定代码、一句中文，以及可选的「过几秒再来」。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Refusal {
    pub status: StatusCode,
    pub code: &'static str,
    pub error: &'static str,
    pub retry_after: Option<u64>,
}

impl Refusal {
    const fn new(status: StatusCode, code: &'static str, error: &'static str) -> Self {
        Self {
            status,
            code,
            error,
            retry_after: None,
        }
    }
    pub(super) const fn after(mut self, secs: u64) -> Self {
        self.retry_after = Some(secs);
        self
    }
}

pub(super) const INVALID: Refusal = Refusal::new(
    StatusCode::BAD_REQUEST,
    "invalid_request",
    "请求格式不合法。",
);
pub(super) const NOT_ALLOWED: Refusal = Refusal::new(
    StatusCode::FORBIDDEN,
    "not_allowed",
    "这个类型没有这个连接器权限。",
);
/// 不存在与名单外同一句话。
pub(super) const NOT_FOUND: Refusal =
    Refusal::new(StatusCode::NOT_FOUND, "not_found", "没有这条工作项。");
pub(super) const NO_ROUTE: Refusal =
    Refusal::new(StatusCode::NOT_FOUND, "no_such_entry", "没有这个入口。");
pub(super) const METHOD: Refusal = Refusal::new(
    StatusCode::METHOD_NOT_ALLOWED,
    "method_not_allowed",
    "这个入口不收这个方法。",
);
pub(super) const STALE: Refusal = Refusal::new(
    StatusCode::CONFLICT,
    "stale_seq",
    "seq 不比上一份大，这一份没有收。",
);
pub(super) const TOO_LARGE: Refusal = Refusal::new(
    StatusCode::PAYLOAD_TOO_LARGE,
    "too_large",
    "快照超过 64 KiB。",
);
pub(super) const INVALID_SESSIONS: Refusal = Refusal::new(
    StatusCode::UNPROCESSABLE_ENTITY,
    "invalid_sessions",
    "字段不合，整份没有收，上一份不变。",
);
pub(super) const TOO_MANY: Refusal = Refusal::new(
    StatusCode::TOO_MANY_REQUESTS,
    "too_many_requests",
    "太快了，稍后再来。",
);
pub(super) const UNAVAILABLE: Refusal = Refusal::new(
    StatusCode::SERVICE_UNAVAILABLE,
    "unavailable",
    "这一刻给不出结论，稍后重试。",
)
.after(30);
pub(super) const STOPPING: Refusal = Refusal::new(
    StatusCode::SERVICE_UNAVAILABLE,
    "stopping",
    "服务正在关停，请稍后重试。",
)
.after(30);
const UNAUTHENTICATED: Refusal = Refusal::new(
    StatusCode::UNAUTHORIZED,
    "unauthenticated",
    "令牌无效或已撤。",
);

impl IntoResponse for Refusal {
    fn into_response(self) -> Response {
        let mut response = (
            self.status,
            Json(serde_json::json!({"code": self.code, "error": self.error})),
        )
            .into_response();
        if let Some(secs) = self.retry_after {
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from(secs));
        }
        response
    }
}

/// 最外层：统一头，并把认证门等处的非 JSON 拒绝换成同一种 JSON。
pub(super) async fn mark(request: Request<Body>, next: Next) -> Response {
    let mut response = next.run(request).await;
    let json = response
        .headers()
        .get(header::CONTENT_TYPE)
        .is_some_and(|value| value.as_bytes().starts_with(b"application/json"));
    if !json && response.status() != StatusCode::NOT_MODIFIED {
        // 认证门的 401 带 challenge，换正文时留住。
        let challenge = response.headers_mut().remove(header::WWW_AUTHENTICATE);
        let refusal = match response.status() {
            StatusCode::UNAUTHORIZED => UNAUTHENTICATED,
            StatusCode::METHOD_NOT_ALLOWED => METHOD,
            StatusCode::NOT_FOUND => NO_ROUTE,
            status if status.is_client_error() => INVALID,
            _ => UNAVAILABLE,
        };
        response = refusal.into_response();
        if let Some(challenge) = challenge {
            response
                .headers_mut()
                .insert(header::WWW_AUTHENTICATE, challenge);
        }
    }
    for (name, value) in [
        (MARKER, "1"),
        (header::CACHE_CONTROL, "no-store"),
        (header::PRAGMA, "no-cache"),
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        (header::REFERRER_POLICY, "no-referrer"),
    ] {
        response
            .headers_mut()
            .insert(name, HeaderValue::from_static(value));
    }
    response
}
