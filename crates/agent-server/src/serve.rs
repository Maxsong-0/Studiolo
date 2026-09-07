//! 组装：白名单只读根 + 既有私有库/账本（no-create）+ 首扫 + 门禁 + rmcp Streamable HTTP，只监听 loopback。

use std::{
    sync::{Arc, Mutex, RwLock},
    time::Duration,
};

use axum::{Router, middleware};
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use studiolo_audit_ledger::SqliteAuditLedger;
use studiolo_core::VaultPath;
use studiolo_supply::{AgentStore, ScanLimits, SupplyRoots};

use crate::{
    auth,
    options::ServeOptions,
    refresh,
    server::AgentSession,
    state::{AppState, build_generation},
};

/// 装配共享状态：白名单只读根、既有私有库与账本（no-create）、首扫代次。
pub fn prepare(options: &ServeOptions) -> Result<Arc<AppState>, String> {
    let allow_refs = options.allow.iter().map(String::as_str).collect::<Vec<_>>();
    let roots = SupplyRoots::open(&options.vault, &allow_refs)
        .map_err(|error| format!("白名单只读根打开失败：{error}"))?;
    let allow = options
        .allow
        .iter()
        .map(VaultPath::parse)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "白名单前缀语法无效".to_owned())?;
    let store = AgentStore::open_existing(&options.data)
        .map_err(|error| format!("agent 私有库：{error}；请先运行 admin init"))?;
    let ledger = SqliteAuditLedger::open_existing(&options.data.join("audit"))
        .map_err(|_| "审计账本不可用或未初始化；请先运行 admin init".to_owned())?;
    let limits = ScanLimits::default();
    let first = build_generation(&roots, limits, 1)?;
    Ok(Arc::new(AppState {
        roots,
        allow,
        store,
        ledger,
        current: RwLock::new(Arc::new(first)),
        outage: Mutex::new(None),
        limits,
    }))
}

/// 路由：`/mcp` 挂 rmcp Streamable HTTP，整个路由树前置类型 token 门禁。
pub fn router(state: Arc<AppState>) -> Router {
    let service_state = state.clone();
    let service = StreamableHttpService::new(
        move || Ok(AgentSession::new(service_state.clone())),
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default(),
    );
    Router::new()
        .route_service("/mcp", service)
        .layer(middleware::from_fn_with_state(state, auth::require_bearer))
}

/// 启动服务直到 Ctrl-C。
pub async fn run(options: ServeOptions) -> Result<(), String> {
    let state = prepare(&options)?;
    let first = state.current();
    let notes = first.snapshot.notes.len();
    let diagnostics = first.snapshot.diagnostics.len();
    drop(first);
    tokio::spawn(refresh::run(state.clone(), Duration::from_secs(60)));
    let app = router(state);
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", options.port))
        .await
        .map_err(|_| format!("无法监听 127.0.0.1:{}", options.port))?;
    println!(
        "studiolo-agent：http://127.0.0.1:{}/mcp（仅 loopback；首扫 {notes} 篇笔记，{diagnostics} 条诊断；Ctrl-C 停止）",
        options.port
    );
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .map_err(|_| "服务异常退出".to_owned())
}
