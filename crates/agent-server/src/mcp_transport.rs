//! MCP 全传输身份绑定、有界初始化、短命会话及流关闭清理。
mod binding;
mod body;
mod gate;
mod input;
mod manager;
mod pool;
mod streams;
mod worker;

use crate::{auth, server::AgentSession, state::AppState};
use axum::{
    Router,
    body::Body,
    http::{HeaderValue, Request, header},
    middleware::{self, Next},
    response::Response,
};
pub(crate) use binding::Binding;
use gate::gate;
use manager::{Manager, REQUEST_BINDING};
use pool::{Limits, Pool};
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use studiolo_supply::audit::{self, Actor};
use tokio::sync::Semaphore;

struct TransportState {
    app: Arc<AppState>,
    manager: Arc<Manager>,
    requests: Arc<Semaphore>,
}

impl TransportState {
    fn valid(&self, binding: &Binding) -> bool {
        binding.live(Instant::now()) && binding.current(&self.app.store)
    }

    fn deny(&self, owner: &str, reason: &str) {
        let actor = Actor {
            agent_type: Some(owner),
            instance_id: None,
        };
        if let Ok(event) = audit::action_denied(actor, "mcp.request", reason) {
            let _ = self.app.audit(event);
        }
    }
}

pub(crate) fn router(app: Arc<AppState>) -> Router {
    with_limits(app, Limits::default())
}

fn with_limits(app: Arc<AppState>, limits: Limits) -> Router {
    assemble(app, limits).0
}

fn assemble(app: Arc<AppState>, limits: Limits) -> (Router, Arc<Manager>) {
    let mut local = LocalSessionManager::default();
    local.session_config.keep_alive = Some(limits.idle);
    local.session_config.init_timeout = Some(limits.initialize);
    // 最终响应本就立即移除通道；取消/注销留下的 completed 项也在下一轮事件前回收。
    local.session_config.completed_cache_ttl = Duration::ZERO;
    let manager = Arc::new(Manager {
        inner: Arc::new(local),
        pool: Arc::new(Pool::new(limits)),
    });
    let state = Arc::new(TransportState {
        app: app.clone(),
        manager: manager.clone(),
        requests: Arc::new(Semaphore::new(16)),
    });
    let service_app = app.clone();
    let service = StreamableHttpService::new(
        move || {
            let binding = REQUEST_BINDING
                .try_with(Arc::clone)
                .map_err(|_| std::io::Error::other("MCP 会话未预占"))?;
            Ok(AgentSession::new_mcp(service_app.clone(), binding))
        },
        manager.clone(),
        StreamableHttpServerConfig::default(),
    );
    // 清理任务只保留 Weak，路由关闭后不会自行延长服务及私有库寿命。
    let weak = Arc::downgrade(&state);
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            let Some(state) = weak.upgrade() else {
                break;
            };
            for binding in state.manager.pool.all() {
                if binding.stale(Instant::now()) || !binding.current(&state.app.store) {
                    state.manager.discard(binding);
                }
            }
        }
    });
    let router = Router::new()
        .route_service("/mcp", service)
        .layer(middleware::from_fn_with_state(state, gate))
        .layer(middleware::from_fn_with_state(app, auth::require_bearer))
        .layer(middleware::from_fn(private));
    (router, manager)
}

async fn private(request: Request<Body>, next: Next) -> Response {
    let mut response = next.run(request).await;
    for (name, value) in [
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

#[cfg(test)]
#[path = "../tests/mcp_transport_private/transport.rs"]
mod tests;
