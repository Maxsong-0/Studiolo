//! 策略平面人侧入口（M4）：desired → 校验 → diff → L3 发布 → effective；版本与回滚；三级吊销。
//!
//! 全部路由挂在会话门禁之内；策略库句柄由可信启动层经 `with_policy_plane` 装上，
//! 缺它时一律 404（与健康页同一形态）。发布动作固定顺序：重验会话 → 审计 Intent →
//! `require_l3` → 策略库事务 → 审计 Outcome；账本不可用即失败关闭，不发布。

mod activate;
mod agents;
mod common;
mod problem;
mod query;
mod reads;
mod write;

use std::sync::Arc;

use axum::{
    Extension, Router,
    routing::{get, post},
};
use studiolo_audit_ledger::AuditLedger;
use studiolo_policy::{DesiredStatus, PolicyStore};
use tokio::sync::Semaphore;

use crate::{AppState, auth_http::policy as input};
use activate::activate;
use query::{desired_list, status, version, versions};
use write::{desired_submit, revoke, validate, withdraw};

pub(crate) const POLICY_CONCURRENCY: u32 = 2;

/// 策略平面句柄与它的审计账本；由可信启动层构造一次。
pub struct PolicyPlane {
    store: PolicyStore,
    identities: Option<studiolo_supply::AgentIdentityView>,
    read_audit: Option<studiolo_supply::read_audit::AgentReadAudit>,
    ledger: Arc<dyn AuditLedger>,
    permits: Arc<Semaphore>,
}

impl PolicyPlane {
    /// 备份与人侧列表使用同一个实际身份源；未装配明确返回 None。
    pub fn checkpoint_agents(
        &self,
    ) -> Result<Option<studiolo_supply::AgentCheckpointAssembly>, studiolo_supply::SupplyError>
    {
        self.identities
            .as_ref()
            .map(studiolo_supply::AgentIdentityView::checkpoint_assembly)
            .transpose()
    }

    /// 参与者与HTTP共用同一许可池、同一策略源；目标由备份协调器注入。
    pub fn checkpoint(
        &self,
        target: std::path::PathBuf,
    ) -> Result<crate::policy_checkpoint::PolicyParticipant, studiolo_policy::PolicyError> {
        Ok(crate::policy_checkpoint::PolicyParticipant::present(
            self.store.checkpoint_handle()?,
            self.permits.clone(),
            target,
        ))
    }

    /// 组合主人发布者句柄与人侧权威账本。
    pub fn new(store: PolicyStore, ledger: Arc<dyn AuditLedger>) -> Self {
        Self {
            store,
            identities: None,
            read_audit: None,
            ledger,
            permits: Arc::new(Semaphore::new(POLICY_CONCURRENCY as usize)),
        }
    }
    /// 显式装配人侧只读身份视图；不授予身份库写能力。
    pub fn with_identities(mut self, identities: studiolo_supply::AgentIdentityView) -> Self {
        self.identities = Some(identities);
        self
    }
    /// 装配接入方权威账本的只读查询，不授予审计追加能力。
    pub fn with_read_audit(mut self, audit: studiolo_supply::read_audit::AgentReadAudit) -> Self {
        self.read_audit = Some(audit);
        self
    }
}

/// 由可信启动层调用：把已打开的策略平面装到整套路由上（只加一层 `Extension`）。
pub fn with_policy_plane(app: Router, plane: Arc<PolicyPlane>) -> Router {
    app.layer(Extension(plane))
}

/// 装配了会话门禁时挂载策略路由；必须在 `security::guard` 那一层之前调用。
pub(crate) fn routes(routes: Router<AppState>) -> Router<AppState> {
    routes
        .route(input::AGENTS, get(agents::list))
        .route(input::READS, post(reads::query))
        .route(input::STATUS, get(status))
        .route(input::DESIRED, get(desired_list).post(desired_submit))
        .route(input::VALIDATE, post(validate))
        .route(input::WITHDRAW, post(withdraw))
        .route(input::ACTIVATE, post(activate))
        .route(input::VERSIONS, get(versions))
        .route("/api/v1/policy/versions/{version}", get(version))
        .route(input::REVOKE, post(revoke))
}

// 让 `DesiredStatus` 等类型在响应里保持库内稳定名，前端按同一枚举解析。
#[allow(dead_code)]
fn _status_names(status: DesiredStatus) -> &'static str {
    status.as_str()
}
