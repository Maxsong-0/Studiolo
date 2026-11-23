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
use studiolo_supply::AgentIdentityView;
use studiolo_task_engine::PersistentTaskEngine;
use studiolo_vault_broker::VaultReader as _;

struct SharedAudit(Arc<dyn AuditLedger>);
impl AuditLedger for SharedAudit {
    fn enter_operation(&self) -> Result<studiolo_audit_ledger::OperationPermit, AuditError> {
        self.0.enter_operation()
    }

    fn try_enter_operation(
        &self,
    ) -> Result<Option<studiolo_audit_ledger::OperationPermit>, AuditError> {
        self.0.try_enter_operation()
    }

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
    identities: AgentIdentityView,
    policy: PolicyAgentStore,
    audit: Arc<dyn AuditLedger>,
    storage: storage::Storage,
    isolated_roots: Vec<std::path::PathBuf>,
    permits: Arc<tokio::sync::Semaphore>,
    /// 跨项目汇总上次看到的审计末条偏移；只是倒扫起点的提示，每次使用前都重新核实。
    overview_tail: std::sync::atomic::AtomicU64,
    /// 黑板列表上次为「提交状态没核对出来」记日志的时刻（毫秒），用来限频。
    commit_check_noted_ms: std::sync::atomic::AtomicI64,
}
impl BoardPlane {
    /// 从实际引擎取得固定 Control 源，不重新开库或执行迁移。
    pub fn checkpoint_source(
        &self,
    ) -> Result<studiolo_control_db::ControlCheckpointSource, String> {
        self.engine
            .try_lock()
            .map_err(|_| "黑板正忙".to_owned())?
            .control_db()
            .checkpoint_source()
            .map_err(|_| "黑板封存源不可用".to_owned())
    }
    /// 只取实际身份视图及其固定附属账本的成对能力。
    pub fn checkpoint_agents(
        &self,
    ) -> Result<studiolo_supply::AgentCheckpointAssembly, studiolo_supply::SupplyError> {
        self.identities.checkpoint_assembly()
    }

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
        let identity_isolated: Vec<_> = isolated_roots
            .iter()
            .map(std::path::PathBuf::as_path)
            .filter(|root| *root != identities)
            .collect();
        let identities = AgentIdentityView::open_existing_isolated(identities, &identity_isolated)
            .map_err(|_| "身份库不可用")?;
        let policy = PolicyAgentStore::open_existing(policy).map_err(|_| "策略库不可用")?;
        let storage = storage::Storage::open(data, isolated)?;
        let control = ControlDb::open_checkpoint_ready(
            storage.file().parent().ok_or("控制目录不可用")?,
            uuid,
        )
        .map_err(|_| "控制库不可用")?;
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
            commit_check_noted_ms: std::sync::atomic::AtomicI64::new(i64::MIN),
        }))
    }
    /// 独立租约巡检；关闭时等待本轮控制事务结束，不留下后台写者。
    pub async fn maintain_until(
        self: Arc<Self>,
        stop: tokio::sync::watch::Receiver<bool>,
    ) -> Result<(), MaintenanceFault> {
        self.maintain_until_with_runner(stop, None).await
    }
    /// 同一维护循环监督可选Runner，先采进度后执行租约对账。
    pub async fn maintain_until_with_runner(
        self: Arc<Self>,
        stop: tokio::sync::watch::Receiver<bool>,
        runner: Option<Arc<RunnerPlane>>,
    ) -> Result<(), MaintenanceFault> {
        self.maintain_every(MAINTENANCE_INTERVAL, MAX_FROZEN, stop, runner)
            .await
    }
    /// 每隔 `interval` 巡检一轮，直到收到停止；冻结以外的故障结束循环。冻结中跳过，但从第一次遇到冻结算起
    /// 连续超过 `max_frozen` 还冻着，就按「备份冻结超过上限」结束。两个时长只有用例会改。
    pub(crate) async fn maintain_every(
        self: Arc<Self>,
        interval: std::time::Duration,
        max_frozen: std::time::Duration,
        mut stop: tokio::sync::watch::Receiver<bool>,
        runner: Option<Arc<RunnerPlane>>,
    ) -> Result<(), MaintenanceFault> {
        // 中间照常巡检过一轮，就重新算。
        let mut frozen_since = None;
        loop {
            tokio::select! {
                _=stop.changed()=>return Ok(()),
                _=tokio::time::sleep(interval)=>{}
            }
            if *stop.borrow() {
                return Ok(());
            }
            let plane = self.clone();
            let runner = runner.clone();
            let tick = tokio::task::spawn_blocking(move || plane.maintain_once(runner.as_deref()))
                .await
                .map_err(|_| MaintenanceFault::Interrupted)??;
            match tick {
                MaintenanceTick::Done => frozen_since = None,
                MaintenanceTick::SkippedFrozen => {
                    let since = *frozen_since.get_or_insert_with(std::time::Instant::now);
                    if since.elapsed() > max_frozen {
                        return Err(MaintenanceFault::FrozenTooLong);
                    }
                    eprintln!("看板租约巡检：备份冻结中，这一轮跳过");
                }
            }
        }
    }
    /// 巡检一轮：先采 Runner 进度，再做租约对账、刷新各项目的投影。须在阻塞线程里、引擎锁之外调用。
    ///
    /// 备份封存正冻结着审计时，这一轮跳过、下一轮照常（BUG-050：以前取许可等满 5 秒报错，
    /// 整个巡检循环结束，人侧照「board 一侧先结束」走完整关停退出）。冻结以外的错误照旧失败关闭。
    pub(crate) fn maintain_once(
        &self,
        runner: Option<&RunnerPlane>,
    ) -> Result<MaintenanceTick, MaintenanceFault> {
        // 冻结中只认这一种「取不到」；准入文件被换、权限被改这类照旧是审计不可用。
        let Some(_operation) = self
            .audit
            .try_enter_operation()
            .map_err(|_| MaintenanceFault::Audit)?
        else {
            return Ok(MaintenanceTick::SkippedFrozen);
        };
        // 下面 Runner 采进度时嵌套取同一把共享许可：手里已有一把，冻结方排不进来，嵌套的不会等。
        if let Some(runner) = runner {
            runner.tick().map_err(|_| MaintenanceFault::Runner)?;
        }
        self.storage
            .check()
            .map_err(|_| MaintenanceFault::Storage)?;
        let mut engine = self.engine.lock().map_err(|_| MaintenanceFault::Engine)?;
        engine
            .reconcile_periodic(jiff::Timestamp::now().as_millisecond())
            .map_err(|_| MaintenanceFault::Reconcile)?;
        let hash = self
            .policy
            .effective()
            .map_err(|_| MaintenanceFault::Policy)?
            .effective_hash;
        let mut projects = std::collections::BTreeSet::new();
        for board in engine
            .control_db()
            .board_tasks()
            .map_err(|_| MaintenanceFault::Control)?
        {
            if let Some(view) = engine
                .board_detail(board.task_id, &hash)
                .map_err(|_| MaintenanceFault::Control)?
            {
                projects.insert(view.task.spec.project);
            }
        }
        // 单个损坏的人工页保留pending，不阻塞其他项目的租约与投影。
        for project in projects {
            let _ = self.project(&mut engine, &project, &hash);
        }
        Ok(MaintenanceTick::Done)
    }
}

/// 租约巡检的间隔。
const MAINTENANCE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(15);
/// 巡检连续遇到冻结的封顶时长（BUG-050）。一次封存从开始到放开冻结都在封存预算之内，再加一分钟余量：
/// 正常的封存再慢也碰不到。封存卡死、冻结一直不放时，巡检按「备份冻结超过上限」结束，人侧走完整关停、
/// 由 systemd 拉起，不会活着却一直写不进去（改前是取许可等满 5 秒就结束）。
pub(crate) const MAX_FROZEN: std::time::Duration =
    std::time::Duration::from_secs(crate::backup::producer::SEAL_BUDGET.as_secs() + 60);

/// 一轮租约巡检的结果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaintenanceTick {
    /// 照常巡检完一轮。
    Done,
    /// 备份封存正冻结着审计：这一轮跳过，下一轮照常（BUG-050）。
    SkippedFrozen,
}

/// 租约巡检结束循环的原因：固定类别，进日志，不带路径、正文和底层错误。
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum MaintenanceFault {
    /// 冻结以外的取许可失败。
    #[error("审计不可用")]
    Audit,
    /// Runner 监管失败。
    #[error("Runner 监管失败")]
    Runner,
    /// 控制目录或控制库被换、权限不对。
    #[error("控制目录或控制库不可用")]
    Storage,
    /// 黑板引擎锁中毒。
    #[error("黑板不可用")]
    Engine,
    /// 租约对账失败。
    #[error("租约对账失败")]
    Reconcile,
    /// 策略库不可用。
    #[error("策略不可用")]
    Policy,
    /// 读控制库里的任务失败。
    #[error("控制库读取失败")]
    Control,
    /// 巡检线程异常退出（panic）。
    #[error("巡检线程异常退出")]
    Interrupted,
    /// 备份冻结一直不放，连续超过 `MAX_FROZEN`（封存卡死）。
    #[error("备份冻结超过上限")]
    FrozenTooLong,
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
