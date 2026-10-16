//! 组装：白名单只读根 + 既有私有库/账本（no-create）+ 首扫 + 门禁 + rmcp Streamable HTTP，只监听 loopback。

use std::{
    sync::{Arc, Mutex, RwLock},
    time::Duration,
};

use axum::Router;
use studiolo_audit_ledger::SqliteAuditLedger;
use studiolo_core::VaultPath;
use studiolo_supply::{AgentStore, ScanLimits, SupplyRoots, replica::ReplicaRoot};

use crate::{
    index::IndexRuntime,
    options::ServeOptions,
    refresh,
    shutdown::{Lifecycle, STOP_LIMIT, Signals},
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
    let policy = options
        .policy
        .as_ref()
        .map(|directory| {
            studiolo_policy::PolicyAgentStore::open_existing(directory)
                .map_err(|error| format!("策略平面私有库：{error}；请先运行 admin policy-init"))
        })
        .transpose()?;
    let board = match (&options.board_socket, &policy) {
        (Some(path), Some(_)) => Some(crate::board_client::BoardClient::new(path)?),
        (Some(_), None) => return Err("黑板入口必须配置有效策略平面".into()),
        (None, _) => None,
    };
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
        policy,
        board,
        ledger,
        current: RwLock::new(Arc::new(first)),
        outage: Mutex::new(None),
        limits,
        index,
        lifecycle: Arc::new(Lifecycle::default()),
    }))
}

/// `/mcp` 与 `/read/*` 使用同一类型门禁；各自负责协议会话，供给操作共用。
pub fn router(state: Arc<AppState>) -> Router {
    crate::mcp_transport::router(state.clone()).merge(crate::http_text::router(state))
}

/// 启动服务直到 Ctrl-C 或 SIGTERM；收到信号后按 [`crate::shutdown`] 的顺序有上限地关停。
pub async fn run(options: ServeOptions) -> Result<(), String> {
    let state = prepare(&options)?;
    let policy_present = state.policy.is_some();
    let first = state.current();
    let notes = first.snapshot.notes.len();
    let diagnostics = first.snapshot.diagnostics.len();
    let index_status = first.index_status;
    drop(first);
    let lifecycle = state.lifecycle.clone();
    let refresh = tokio::spawn(refresh::run(state.clone(), Duration::from_secs(60)));
    let app = router(state);
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", options.port))
        .await
        .map_err(|_| format!("无法监听 127.0.0.1:{}", options.port))?;
    // 宣布监听之前装好信号处理器，之后的 SIGTERM 一律走优雅关停；原先要等关停 future 第一次被 poll
    // 才装，这个窗口里的 SIGTERM 按默认动作直接杀进程（BUG-033）。也不提前到首扫之前：首扫卡住时，
    // 装好的处理器会把 SIGTERM 拖到首扫做完，那时还没有连接要收尾，按默认动作立即退出更好。
    let signals = Signals::listen();
    println!(
        "studiolo-agent：http://127.0.0.1:{}/mcp（仅 loopback；首扫 {notes} 篇笔记，{diagnostics} 条诊断；Ctrl-C 或 SIGTERM 停止）",
        options.port
    );
    println!("派生索引：{index_status:?}");
    println!(
        "策略平面：{}",
        if policy_present {
            "已接入（能力集∩部署上限、三级吊销复核、policy_read／propose_desired）"
        } else {
            "未接入（M3 兼容模式：白名单只来自 --allow，无能力集与吊销表）"
        }
    );
    let stopping = lifecycle.clone();
    let server = axum::serve(listener, app).with_graceful_shutdown(async move {
        signals.received().await;
        // 先置关停：门禁从这一刻起拒绝新会话和新请求；axum 随即停止 accept，空闲连接关掉，
        // MCP 传输排空已进门的请求后关闭全部会话，SSE 流随之结束。
        stopping.stop();
    });
    let wind_down = {
        let lifecycle = lifecycle.clone();
        async move {
            let served = server.await;
            // 服务自己出错退出时没经过信号，也要让后台重扫停下。
            lifecycle.stop();
            // 正在跑的那次重扫在阻塞线程上写派生缓存，做完再返回。
            let _ = refresh.await;
            served
        }
    };
    let deadline = async {
        lifecycle.stopped().await;
        tokio::time::sleep(STOP_LIMIT).await;
    };
    tokio::select! {
        served = wind_down => served.map_err(|_| "服务异常退出".to_owned()),
        () = deadline => Err(format!(
            "关停超时：收到停止信号 {} 秒后仍有连接或后台重扫没有结束，强制退出",
            STOP_LIMIT.as_secs()
        )),
    }
}
