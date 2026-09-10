//! HTTP 只读文本外壳：类型门禁、有界短命会话；供给实现与 MCP 完全共用。
mod input;
mod sessions;

use crate::{
    auth::{self, AgentType},
    server::{AgentSession, ListEntriesArgs, ReadNoteArgs, SearchArgs},
    state::AppState,
};
use axum::{
    Router,
    body::Body,
    extract::State,
    http::{HeaderValue, Method, Request, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::any,
};
use input::{INSTANCE_HEADER, SESSION_HEADER};
use rmcp::{ErrorData, model::ErrorCode};
use sessions::{ReserveError, Sessions};
use std::{sync::Arc, time::Instant};
use studiolo_supply::audit::{self, Actor};
use tokio::sync::Semaphore;

struct HttpState {
    app: Arc<AppState>,
    sessions: Sessions<AgentSession>,
    requests: Semaphore,
}

pub(crate) fn router(app: Arc<AppState>) -> Router {
    let state = Arc::new(HttpState {
        app: app.clone(),
        sessions: Sessions::new(),
        requests: Semaphore::new(16),
    });
    let routes = Router::new()
        .route("/handshake", any(dispatch))
        .route("/entries", any(dispatch))
        .route("/note", any(dispatch))
        .route("/search", any(dispatch))
        .fallback(dispatch)
        .with_state(state)
        .layer(middleware::from_fn_with_state(app, auth::require_bearer))
        .layer(middleware::from_fn(plain_private));
    Router::new().nest("/read", routes)
}

/// 覆盖成功、认证拒绝、参数错误及 404/405；正文始终作为纯文本传输。
async fn plain_private(request: Request<Body>, next: Next) -> Response {
    let mut response = next.run(request).await;
    for (name, value) in [
        (header::CONTENT_TYPE, "text/plain; charset=utf-8"),
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

fn failure(status: StatusCode, message: &'static str) -> Response {
    let mut response = (status, message).into_response();
    if status == StatusCode::UNAUTHORIZED {
        response.headers_mut().insert(
            header::WWW_AUTHENTICATE,
            HeaderValue::from_static("Bearer realm=\"studiolo-agent\""),
        );
    }
    response
}

fn busy(error: ReserveError) -> Response {
    if error == ReserveError::Unavailable {
        return failure(StatusCode::SERVICE_UNAVAILABLE, "会话管理不可用");
    }
    let mut response = failure(
        StatusCode::TOO_MANY_REQUESTS,
        "HTTP请求或会话名额已满，请稍后重试",
    );
    response
        .headers_mut()
        .insert(header::RETRY_AFTER, HeaderValue::from_static("60"));
    response
}

fn supply_error(state: &HttpState, error: ErrorData) -> Response {
    if error.message.contains("审计不可用") {
        return match state.app.degraded() {
            Some(outage) => (
                StatusCode::SERVICE_UNAVAILABLE,
                format!(
                    "审计不可用：outage {}。本次读写全部拒绝，不提供路牌或内容。",
                    outage.id
                ),
            )
                .into_response(),
            None => failure(
                StatusCode::SERVICE_UNAVAILABLE,
                "审计不可用，本次读写全部拒绝",
            ),
        };
    }
    let (status, message) = match error.code {
        ErrorCode::INVALID_PARAMS => (StatusCode::BAD_REQUEST, "请求参数或白名单路径无效"),
        ErrorCode::RESOURCE_NOT_FOUND => (StatusCode::NOT_FOUND, "笔记不存在"),
        ErrorCode::INVALID_REQUEST => (StatusCode::UNAUTHORIZED, "会话无效，请重新握手"),
        _ => (
            StatusCode::SERVICE_UNAVAILABLE,
            "供给暂不可用，本次拒绝内容",
        ),
    };
    failure(status, message)
}

fn deny(state: &HttpState, agent_type: &str, reason: &str) {
    let actor = Actor {
        agent_type: Some(agent_type),
        instance_id: None,
    };
    if let Ok(event) = audit::action_denied(actor, "http.request", reason) {
        let _ = state.app.audit(event);
    }
}

async fn dispatch(State(state): State<Arc<HttpState>>, request: Request<Body>) -> Response {
    let Some(AgentType(agent_type)) = request.extensions().get::<AgentType>() else {
        return failure(StatusCode::UNAUTHORIZED, "未认证");
    };
    if request.method() != Method::GET {
        deny(&state, agent_type, "method_not_allowed");
        let mut response = failure(StatusCode::METHOD_NOT_ALLOWED, "只支持GET");
        response
            .headers_mut()
            .insert(header::ALLOW, HeaderValue::from_static("GET"));
        return response;
    }
    let path = request.uri().path();
    let (allowed, required): (&[&str], &[&str]) = match path {
        "/handshake" => (&[], &[]),
        "/entries" => (&["project"], &["project"]),
        "/note" => (&["path"], &["path"]),
        "/search" => (&["project", "text", "mode"], &["project", "text"]),
        _ => return failure(StatusCode::NOT_FOUND, "入口不存在"),
    };
    let Ok(mut query) = input::query(request.uri().query(), allowed, required) else {
        deny(&state, agent_type, "invalid_query");
        return failure(StatusCode::BAD_REQUEST, "查询参数无效");
    };
    let Ok(id) = input::session_id(request.headers()) else {
        deny(&state, agent_type, "invalid_session_header");
        return failure(StatusCode::BAD_REQUEST, "会话头无效");
    };
    let Ok(_permit) = state.requests.try_acquire() else {
        return busy(ReserveError::Full);
    };
    let (id, session, text, reservation) = if let Some(id) = id {
        let Some(session) = state.sessions.get(&id, agent_type, Instant::now()) else {
            deny(&state, agent_type, "invalid_session");
            return failure(StatusCode::UNAUTHORIZED, "会话无效或过期，请重新握手");
        };
        if session.bound_type() != Some(agent_type.as_str()) {
            return failure(StatusCode::UNAUTHORIZED, "会话身份不匹配");
        }
        let supplied = match path {
            "/handshake" => session.text_handshake().await,
            "/entries" => {
                session
                    .text_list_entries(ListEntriesArgs {
                        project: query.remove("project").unwrap_or_default(),
                    })
                    .await
            }
            "/note" => {
                session
                    .text_read_note(ReadNoteArgs {
                        path: query.remove("path").unwrap_or_default(),
                    })
                    .await
            }
            _ => {
                session
                    .text_search(SearchArgs {
                        project: query.remove("project").unwrap_or_default(),
                        text: query.remove("text").unwrap_or_default(),
                        mode: query.remove("mode"),
                    })
                    .await
            }
        };
        let text = match supplied {
            Ok(text) => text,
            Err(error) => return supply_error(&state, error),
        };
        (id, session, text, None)
    } else {
        if path != "/handshake" {
            deny(&state, agent_type, "missing_session");
            return failure(
                StatusCode::UNAUTHORIZED,
                "请先握手并携带Studiolo-Session-Id",
            );
        }
        let reservation = match state.sessions.reserve(agent_type, Instant::now()) {
            Ok(reservation) => reservation,
            Err(error) => return busy(error),
        };
        let session = Arc::new(AgentSession::new_http(state.app.clone()));
        let text = match session.initialize_text(agent_type).await {
            Ok(text) => text,
            Err(error) => return supply_error(&state, error),
        };
        (
            reservation.id().to_owned(),
            session,
            text,
            Some(reservation),
        )
    };
    if !state.app.store.type_is_active(agent_type).unwrap_or(false) {
        return failure(StatusCode::UNAUTHORIZED, "会话已失效，请重新握手");
    }
    let Some(instance) = session.bound_instance() else {
        return failure(StatusCode::SERVICE_UNAVAILABLE, "实例不可用");
    };
    let (Ok(session_header), Ok(instance_header)) = (
        HeaderValue::from_str(&id),
        HeaderValue::from_str(&instance.id),
    ) else {
        return failure(StatusCode::SERVICE_UNAVAILABLE, "实例不可用");
    };
    let mut response = text.into_response();
    response
        .headers_mut()
        .insert(SESSION_HEADER, session_header);
    response
        .headers_mut()
        .insert(INSTANCE_HEADER, instance_header);
    // 全部响应准备成功后才发布：此前的失败与取消仍由预留守卫清理。
    if let Some(reservation) = reservation {
        if let Err(error) = reservation.commit(session, Instant::now()) {
            return busy(error);
        }
    } else if !state.sessions.touch(&id, agent_type, Instant::now()) {
        return failure(StatusCode::UNAUTHORIZED, "会话已失效，请重新握手");
    }
    response
}
