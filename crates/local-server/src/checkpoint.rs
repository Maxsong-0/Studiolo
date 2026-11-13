//! 七个实际 owner 的总窗口：先排空完整操作，再按固定顺序冻结；全体原事件核对后才导出。
mod export;
use crate::{
    CanonicalWorkbench, FrozenWorkbench, PolicyPlane,
    board::BoardPlane,
    policy_checkpoint::PolicyParticipant,
    sync_plane::{SyncPlane, checkpoint::FrozenSyncPlane},
};
pub use export::{CheckpointExports, OwnerExportPaths};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use studiolo_audit_ledger::{
    AuditCheckpointAccess, AuditEvent, AuditQuiescence, FrozenAudit, SqliteAuditLedger,
};
use studiolo_auth::{AuthCheckpointSource, FrozenAuth, SessionManager};
use studiolo_backup::{
    checkpoint::manifest::AuditCoverageProof,
    worker::{job::backup_intent, recovery_point::BackupTargetKind, state::JobKey},
};
use studiolo_control_db::{ControlCheckpointSource, FrozenControl};
use studiolo_core::checkpoint::{
    CheckpointError, CheckpointOwner as Owner, CheckpointParticipant, FreezeRequest,
};
use studiolo_supply::{AgentCheckpointAssembly, FrozenAgentStore};
type Result<T> = std::result::Result<T, CheckpointError>;

/// 仅由可信启动层交入实际装配，已装配能力不得伪装 None。
pub struct CheckpointInputs {
    /// 正在服务的唯一 canonical 工作台。
    pub workbench: Arc<CanonicalWorkbench>,
    /// 正在服务的 Auth 管理器及原主人绑定。
    pub sessions: Arc<SessionManager>,
    /// 人侧实际权威账本；也提供完整操作门禁。
    pub audit: Arc<SqliteAuditLedger>,
    /// 实际装配的黑板；None 仅表示未装配。
    pub board: Option<Arc<BoardPlane>>,
    /// 实际装配的双向同步平面。
    pub bridge: Option<Arc<SyncPlane>>,
    /// 实际身份库与其固定附属审计链。
    pub agents: Option<AgentCheckpointAssembly>,
    /// 实际主人策略发布平面及共享许可池。
    pub policy: Option<Arc<PolicyPlane>>,
}
/// 固定正在服务的源能力；无任意路径开库、初始化或取消在途任务的方法。
pub struct CheckpointOwners {
    workbench: Arc<CanonicalWorkbench>,
    auth: AuthCheckpointSource,
    audit: AuditCheckpointAccess,
    control: Option<ControlCheckpointSource>,
    bridge: Option<Arc<SyncPlane>>,
    agents: Option<AgentCheckpointAssembly>,
    policy: Option<Arc<PolicyPlane>>,
}
/// 字段按逆冻结顺序析构；SQL/唯一 owner 先释放，最后才恢复人侧和接入方完整操作准入。
pub struct FrozenOwners {
    audit: FrozenAudit,
    agent_audit: Option<FrozenAudit>,
    policy: Option<PolicyParticipant>,
    agents: Option<FrozenAgentStore>,
    bridge: Option<FrozenSyncPlane>,
    auth: FrozenAuth,
    control: Option<FrozenControl>,
    broker: FrozenWorkbench,
    _human_quiet: AuditQuiescence,
    _agent_quiet: Option<AuditQuiescence>,
    request: FreezeRequest,
    paths: OwnerExportPaths,
    intent: AuditEvent,
    coverage: Option<AuditCoverageProof>,
    deadline: Instant,
}
impl CheckpointOwners {
    /// 在暂停业务前从实际装配固定全部源能力；黑板不能遗漏其身份库。
    pub fn new(input: CheckpointInputs) -> Result<Self> {
        if input.board.is_some() && input.agents.is_none() {
            return Err(unavailable(Owner::AgentStore));
        }
        Ok(Self {
            auth: input
                .sessions
                .checkpoint_source()
                .map_err(|_| unavailable(Owner::Auth))?,
            audit: AuditCheckpointAccess::from_ledger(input.audit)
                .map_err(|_| unavailable(Owner::Audit))?,
            control: input
                .board
                .as_ref()
                .map(|plane| {
                    plane
                        .checkpoint_source()
                        .map_err(|_| unavailable(Owner::Control))
                })
                .transpose()?,
            workbench: input.workbench,
            bridge: input.bridge,
            agents: input.agents,
            policy: input.policy,
        })
    }
    /// Intent 必须已由独立作业方落账；整个窗口共用一个原截止时间，任何失败按逆序释放。
    pub fn freeze(
        &self,
        request: FreezeRequest,
        key: &JobKey,
        target: BackupTargetKind,
        paths: OwnerExportPaths,
    ) -> Result<FrozenOwners> {
        if request.checkpoint_id != key.checkpoint_id {
            return Err(unavailable(Owner::Audit));
        }
        let intent = backup_intent(key, target).map_err(|_| unavailable(Owner::Audit))?;
        let deadline = Instant::now() + Duration::from_millis(u64::from(request.budget.millis()));
        // 接入方可嵌套调用人侧黑板，反向无完整操作嵌套：必须先排空接入方。
        let agent_quiet = self
            .agents
            .as_ref()
            .map(|agent| {
                agent
                    .quiesce(deadline)
                    .map_err(|_| unavailable(Owner::AgentStore))
            })
            .transpose()?;
        let human_quiet = self
            .audit
            .quiesce(deadline)
            .map_err(|_| unavailable(Owner::Audit))?;
        check_deadline(deadline, Owner::Broker)?;
        let broker = self
            .workbench
            .freeze_checkpoint(deadline)
            .map_err(|_| unavailable(Owner::Broker))?;
        let control = self
            .control
            .as_ref()
            .map(|source| {
                source
                    .freeze(deadline)
                    .map_err(|_| unavailable(Owner::Control))
            })
            .transpose()?;
        let auth = self
            .auth
            .freeze(deadline)
            .map_err(|_| unavailable(Owner::Auth))?;
        let bridge = self
            .bridge
            .as_ref()
            .map(|plane| {
                plane
                    .freeze_checkpoint(deadline)
                    .map_err(|_| unavailable(Owner::Bridge))
            })
            .transpose()?;
        let agents = self
            .agents
            .as_ref()
            .map(|source| {
                source
                    .freeze_store(deadline)
                    .map_err(|_| unavailable(Owner::AgentStore))
            })
            .transpose()?;
        let policy = self
            .policy
            .as_ref()
            .map(|plane| {
                plane
                    .checkpoint(paths.policy.clone())
                    .map(|p| p.before(deadline))
                    .map_err(|_| unavailable(Owner::Policy))
            })
            .transpose()?;
        if let Some(participant) = &policy {
            participant.freeze(&request)?;
        }
        // 两条链都属于最后 Audit 阶段；先冻结接入方，再冻结人侧，析构次序相反。
        let agent_audit = self
            .agents
            .as_ref()
            .map(|source| {
                source
                    .freeze_audit(deadline)
                    .map_err(|_| unavailable(Owner::Audit))
            })
            .transpose()?;
        let audit = self
            .audit
            .freeze(deadline)
            .map_err(|_| unavailable(Owner::Audit))?;
        let mut frozen = FrozenOwners {
            audit,
            agent_audit,
            policy,
            agents,
            bridge,
            auth,
            control,
            broker,
            _human_quiet: human_quiet,
            _agent_quiet: agent_quiet,
            request,
            paths,
            intent,
            coverage: None,
            deadline,
        };
        frozen.verify_audit()?;
        Ok(frozen)
    }
}
impl FrozenOwners {
    fn verify_audit(&mut self) -> Result<()> {
        self.coverage = None;
        self.audit
            .verify_event(&self.intent)
            .map_err(|_| unavailable(Owner::Audit))?;
        // 只有实际核验成功才累计，包括本作业在副作用前已落账的 Intent。
        let mut checked = 1_u64;
        add(
            &mut checked,
            self.broker
                .verify_audit(&self.audit)
                .map_err(|_| unavailable(Owner::Broker))?
                .checked(),
        )?;
        if let Some(control) = &mut self.control {
            add(
                &mut checked,
                control
                    .verify_audit(&self.audit)
                    .map_err(|_| unavailable(Owner::Control))?
                    .checked(),
            )?;
        }
        add(
            &mut checked,
            self.auth
                .verify_audit(&self.audit)
                .map_err(|_| unavailable(Owner::Auth))?
                .checked(),
        )?;
        if let Some(bridge) = &mut self.bridge {
            add(
                &mut checked,
                bridge
                    .verify_audit(&self.audit)
                    .map_err(|_| unavailable(Owner::Bridge))?,
            )?;
        }
        match (&mut self.agents, &self.agent_audit) {
            (Some(agent), Some(audit)) => add(
                &mut checked,
                agent
                    .verify_audit(audit)
                    .map_err(|_| unavailable(Owner::AgentStore))?
                    .checked(),
            )?,
            (None, None) => {}
            _ => return Err(unavailable(Owner::AgentStore)),
        }
        check_deadline(self.deadline, Owner::Audit)?;
        self.coverage = Some(AuditCoverageProof {
            acked_events_checked: checked,
            acked_events_matched: checked,
            offset_comparison_only: false,
        });
        Ok(())
    }
}
fn add(total: &mut u64, count: u64) -> Result<()> {
    *total = total
        .checked_add(count)
        .ok_or_else(|| unavailable(Owner::Audit))?;
    Ok(())
}
fn unavailable(owner: Owner) -> CheckpointError {
    CheckpointError::OwnerUnavailable { owner }
}
fn check_deadline(deadline: Instant, owner: Owner) -> Result<()> {
    if Instant::now() >= deadline {
        Err(CheckpointError::FreezeTimeout { owner })
    } else {
        Ok(())
    }
}
