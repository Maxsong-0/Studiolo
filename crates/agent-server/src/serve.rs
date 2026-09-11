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
use studiolo_supply::{AgentStore, ScanLimits, SupplyRoots, replica::ReplicaRoot};

use crate::{
    auth,
    index::IndexRuntime,
    options::ServeOptions,
    refresh,
    server::AgentSession,
    state::{AppState, Generation, SupplySource, now},
};

/// 装配共享状态：白名单只读根、既有私有库与账本（no-create）、首扫代次。
pub fn prepare(options: &ServeOptions) -> Result<Arc<AppState>, String> {
    let allow_refs = options.allow.iter().map(String::as_str).collect::<Vec<_>>();
    let source = match (&options.vault, &options.replica_root) {
        (Some(vault), None) => SupplySource::Direct(
            SupplyRoots::open(vault, &allow_refs)
                .map_err(|error| format!("白名单只读根打开失败：{error}"))?,
        ),
        (None, Some(replica)) => SupplySource::Replica(
            ReplicaRoot::open(replica, &options.allow, options.replica_max_age_secs)
                .map_err(|error| error.to_string())?,
        ),
        _ => return Err("必须且只能指定一种供给源".to_owned()),
    };
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
    let (snapshot, freshness) = source.scan(limits, None)?;
    let index = IndexRuntime::open(options)?;
    let (model, index_status) = index.build(&snapshot, None)?;
    let first = Generation {
        number: 1,
        snapshot,
        model,
        index_status,
        generated_at: now(),
        freshness,
    };
    Ok(Arc::new(AppState {
        source,
        scan_lock: Mutex::new(()),
        allow,
        store,
        ledger,
        current: RwLock::new(Arc::new(first)),
        outage: Mutex::new(None),
        limits,
        index,
    }))
}

/// `/mcp` 与 `/read/*` 使用同一类型门禁；各自负责协议会话，供给操作共用。
pub fn router(state: Arc<AppState>) -> Router {
    let service_state = state.clone();
    let service = StreamableHttpService::new(
        move || Ok(AgentSession::new(service_state.clone())),
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default(),
    );
    Router::new()
        .route_service("/mcp", service)
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth::require_bearer,
        ))
        .merge(crate::http_text::router(state))
}

/// 启动服务直到 Ctrl-C 或 SIGTERM。
pub async fn run(options: ServeOptions) -> Result<(), String> {
    let state = prepare(&options)?;
    let first = state.current();
    let notes = first.snapshot.notes.len();
    let diagnostics = first.snapshot.diagnostics.len();
    let index_status = first.index_status;
    drop(first);
    tokio::spawn(refresh::run(state.clone(), Duration::from_secs(60)));
    let app = router(state);
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", options.port))
        .await
        .map_err(|_| format!("无法监听 127.0.0.1:{}", options.port))?;
    println!(
        "studiolo-agent：http://127.0.0.1:{}/mcp（仅 loopback；首扫 {notes} 篇笔记，{diagnostics} 条诊断；Ctrl-C 或 SIGTERM 停止）",
        options.port
    );
    println!("派生索引：{index_status:?}");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(|_| "服务异常退出".to_owned())
}

/// Ctrl-C（SIGINT）与 systemd 停止（SIGTERM）都触发优雅关停；SIGTERM 注册失败时只等待 Ctrl-C。
async fn shutdown_signal() {
    let interrupt = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut stream) => {
                let _ = stream.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = interrupt => {}
        () = terminate => {}
    }
}
