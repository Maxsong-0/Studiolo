//! M5 黑板组合面：复用 canonical 单写者，身份与策略每次复核，运行正文不进 Control DB。
mod auth;
mod content;
mod execute;
mod execution_http;
mod executor;
mod http;
mod overview;
mod owner;
mod replay;
mod requests;
pub use executor::{
    CodeLandingResult, CodeLandingReviewGuard, PolicyConfig, ProjectConfig, ProviderConfig,
    RunSummary, RunnerConfig, RunnerPlane, SourceConfig, StartRequest,
};
mod storage;
mod timeline;

use crate::CanonicalWorkbench;
use std::{
    path::Path,
    sync::{Arc, Mutex},
};
use studiolo_audit_ledger::{AuditError, AuditEvent, AuditFilter, AuditLedger, AuditRecord};
use studiolo_control_db::ControlDb;
use studiolo_policy::PolicyAgentStore;
use studiolo_supply::AgentStore;
use studiolo_task_engine::PersistentTaskEngine;
use studiolo_vault_broker::VaultReader as _;

struct SharedAudit(Arc<dyn AuditLedger>);
impl AuditLedger for SharedAudit {
    fn append(&self, event: AuditEvent) -> Result<AuditRecord, AuditError> {
        self.0.append(event)
    }
    fn query(&self, filter: &AuditFilter) -> Result<Vec<AuditRecord>, AuditError> {
        self.0.query(filter)
    }
}
type Engine = PersistentTaskEngine<owner::SharedBroker, SharedAudit>;

/// 已显式装配的黑板平面；没有可从 HTTP 构造的认证旁路。
pub struct BoardPlane {
    engine: Mutex<Engine>,
    identities: AgentStore,
    policy: PolicyAgentStore,
    audit: Arc<dyn AuditLedger>,
    storage: storage::Storage,
    isolated_roots: Vec<std::path::PathBuf>,
    permits: Arc<tokio::sync::Semaphore>,
    /// 跨项目汇总上次看到的审计末条偏移；只是倒扫起点的提示，每次使用前都重新核实。
    overview_tail: std::sync::atomic::AtomicU64,
}
impl BoardPlane {
    /// 已有 canonical 写者、身份库、策略与审计全部就绪后，才打开独立控制目录。
    pub fn open(
        workbench: Arc<CanonicalWorkbench>,
        data: &Path,
        identities: &Path,
        policy: &Path,
        isolated: &[&Path],
        audit: Arc<dyn AuditLedger>,
    ) -> Result<Arc<Self>, String> {
        Self::open_with_runner(workbench, data, identities, policy, isolated, audit, None)
    }
    /// Runner恢复必须先隔离旧作用域，再进行黑板租约对账。
    #[allow(clippy::too_many_arguments)]
    pub fn open_with_runner(
        workbench: Arc<CanonicalWorkbench>,
        data: &Path,
        identities: &Path,
        policy: &Path,
        isolated: &[&Path],
        audit: Arc<dyn AuditLedger>,
        runner: Option<&RunnerConfig>,
    ) -> Result<Arc<Self>, String> {
        let broker = owner::SharedBroker::new(workbench).map_err(|_| "canonical 不可用")?;
        let uuid = broker.vault_uuid();
        let isolated_roots = isolated
            .iter()
            .map(|p| p.to_path_buf())
            .chain([
                data.to_path_buf(),
                identities.to_path_buf(),
                policy.to_path_buf(),
            ])
            .collect::<Vec<_>>();
        let identities = AgentStore::open_existing(identities).map_err(|_| "身份库不可用")?;
        let policy = PolicyAgentStore::open_existing(policy).map_err(|_| "策略库不可用")?;
        let storage = storage::Storage::open(data, isolated)?;
        let control = ControlDb::open(storage.file(), uuid).map_err(|_| "控制库不可用")?;
        let mut engine = PersistentTaskEngine::new(control, broker, SharedAudit(audit.clone()));
        if let Some(config) = runner {
            executor::isolate_persisted(engine.control_db(), config, &isolated_roots)?;
        }
        for board in engine
            .control_db()
            .board_tasks()
            .map_err(|_| "控制库不可用")?
        {
            if let Some(task) = engine
                .control_db()
                .task(board.task_id)
                .map_err(|_| "控制库不可用")?
                && let Some(attempt_id) = task.active_attempt
                && let Some(attempt) = engine
                    .control_db()
                    .attempt(board.task_id, attempt_id)
                    .map_err(|_| "控制库不可用")?
                && attempt.runner_ref_json.is_some()
                && runner.is_none()
            {
                return Err("先隔离旧执行体并完成恢复，才能启动黑板".into());
            }
        }
        engine
            .reconcile_startup(jiff::Timestamp::now().as_millisecond())
            .map_err(|_| "黑板启动恢复失败")?;
        Ok(Arc::new(Self {
            engine: Mutex::new(engine),
            identities,
            policy,
            audit,
            storage,
            isolated_roots,
            permits: Arc::new(tokio::sync::Semaphore::new(4)),
            overview_tail: std::sync::atomic::AtomicU64::new(0),
        }))
    }
    /// 独立租约巡检；关闭时等待本轮控制事务结束，不留下后台写者。
    pub async fn maintain_until(
        self: Arc<Self>,
        stop: tokio::sync::watch::Receiver<bool>,
    ) -> Result<(), String> {
        self.maintain_until_with_runner(stop, None).await
    }
    /// 同一维护循环监督可选Runner，先采进度后执行租约对账。
    pub async fn maintain_until_with_runner(
        self: Arc<Self>,
        mut stop: tokio::sync::watch::Receiver<bool>,
        runner: Option<Arc<RunnerPlane>>,
    ) -> Result<(), String> {
        loop {
            tokio::select! {
                _=stop.changed()=>return Ok(()),
                _=tokio::time::sleep(std::time::Duration::from_secs(15))=>{}
            }
            if *stop.borrow() {
                return Ok(());
            }
            let plane = self.clone();
            let runner = runner.clone();
            tokio::task::spawn_blocking(move || {
                if let Some(runner) = runner {
                    runner.tick()?;
                }
                plane.storage.check().map_err(str::to_owned)?;
                let mut engine = plane.engine.lock().map_err(|_| "黑板不可用".to_owned())?;
                engine
                    .reconcile_periodic(jiff::Timestamp::now().as_millisecond())
                    .map_err(|e| e.to_string())?;
                let hash = plane
                    .policy
                    .effective()
                    .map_err(|_| "策略不可用".to_owned())?
                    .effective_hash;
                let mut projects = std::collections::BTreeSet::new();
                for board in engine
                    .control_db()
                    .board_tasks()
                    .map_err(|e| e.to_string())?
                {
                    if let Some(view) = engine
                        .board_detail(board.task_id, &hash)
                        .map_err(|e| e.to_string())?
                    {
                        projects.insert(view.task.spec.project);
                    }
                }
                // 单个损坏的人工页保留pending，不阻塞其他项目的租约与投影。
                for project in projects {
                    let _ = plane.project(&mut engine, &project, &hash);
                }
                Ok::<_, String>(())
            })
            .await
            .map_err(|_| "黑板巡检中断".to_owned())??;
        }
    }
}

/// 人侧监督面只读路由，必须挂在既有会话门禁内。
pub(crate) fn routes(routes: axum::Router<crate::AppState>) -> axum::Router<crate::AppState> {
    execution_http::routes(http::routes(routes))
}
/// 将已验证的黑板装配至既有会话路由。
pub fn with_board_plane(app: axum::Router, plane: Arc<BoardPlane>) -> axum::Router {
    app.layer(axum::Extension(plane))
}
/// 私有 Unix 路由单独监听；不合并进公网路由。
pub fn private_board_router(plane: Arc<BoardPlane>) -> axum::Router {
    http::private_router(plane)
}
