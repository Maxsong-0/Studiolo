//! 七个实际 owner 的总窗口：先排空完整操作，再按固定顺序冻结；全体原事件核对后才导出。
//!
//! 冻结窗口只定格（BUG-045）：窗口里各库和写者的两棵树定在同一刻、核本作业的 Intent 和 Bridge 的回执、
//! 导出有固定上限的小状态，然后按逆序放开全部屏障（[`FrozenOwners::release`]）；随数据涨的回执核对
//! （写者的也在内，BUG-050）、拷贝、整本验链、过滤视图和发布都在放开之后从定格做（[`ReleasedOwners`]），
//! 写请求不再等它们。
mod assembly;
mod export;
mod released;
mod restore;
mod settle;
mod steps;
use crate::{
    CanonicalWorkbench, FrozenWorkbench, PolicyPlane,
    board::BoardPlane,
    policy_checkpoint::PolicyParticipant,
    sync_plane::{SyncPlane, checkpoint::FrozenSyncPlane},
};
pub use assembly::CheckpointInputs;
pub use export::{CheckpointExports, CheckpointTimes, OwnerExportPaths, trusted_catalog};
pub use released::{ReleasedOwners, SealFailure};
pub use restore::{
    Cutover, CutoverRefusal, Journal, NewRootRestore, RestoreRefusal, RestoreReport,
    RevocationPending, begin_cutover, finish_cutover, require_restore_revoked,
    restore_into_new_root, restore_started,
};
/// 恢复动手之前的两个纯判断，给协调器用例直接喂值测。
#[cfg(test)]
pub(crate) use restore::{Found, classify, plan_from};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
pub use steps::StepTimes;
use studiolo_audit_ledger::{
    AuditCheckpointAccess, AuditEvent, AuditQuiescence, FrozenAudit, SqliteAuditLedger,
};
use studiolo_auth::{AuthCheckpointSource, FactorRuntime, FrozenAuth, SessionManager};
use studiolo_backup::worker::{
    job::backup_intent, recovery_point::BackupTargetKind, state::JobKey,
};
use studiolo_control_db::{ControlCheckpointSource, FrozenControl};
use studiolo_core::checkpoint::{
    CheckpointError, CheckpointOwner as Owner, CheckpointParticipant, FreezeRequest,
};
use studiolo_supply::{AgentCheckpointAssembly, FrozenAgentStore};
type Result<T> = std::result::Result<T, CheckpointError>;

/// 固定正在服务的源能力；无任意路径开库、初始化或取消在途任务的方法。
pub struct CheckpointOwners {
    workbench: Arc<CanonicalWorkbench>,
    auth: AuthCheckpointSource,
    /// 冻结之前补记登录库待发审计用的原实例（`settle` 子模块）；冻结本身只用上面的固定源。
    sessions: Arc<SessionManager>,
    factors: Option<Arc<FactorRuntime>>,
    audit: AuditCheckpointAccess,
    control: Option<ControlCheckpointSource>,
    bridge: Option<Arc<SyncPlane>>,
    agents: Option<AgentCheckpointAssembly>,
    policy: Option<Arc<PolicyPlane>>,
}
/// 字段按逆冻结顺序析构；SQL/唯一 owner 先释放，最后才恢复人侧和接入方完整操作准入。
/// 正常路径不靠析构：[`FrozenOwners::release`] 在窗口里收尾后按同一顺序放开，交出 [`ReleasedOwners`]。
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
    /// 窗口里核过的回执数（本作业的 Intent、Bridge 与传输）；写者和其余各库的回执在放开之后核。
    checked: Option<u64>,
    deadline: Instant,
}
impl CheckpointOwners {
    /// 在暂停业务前从实际装配固定全部源能力；黑板不能遗漏其身份库。
    pub fn new(input: CheckpointInputs) -> Result<Self> {
        let agents = input.agents()?;
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
            sessions: input.sessions,
            factors: input.factors,
            bridge: input.bridge,
            agents,
            policy: input.policy,
        })
    }
    /// 不记分步的冻结，只给用例用；生产封存用 [`Self::freeze_timed`]。
    #[cfg(test)]
    pub(crate) fn freeze(
        &self,
        request: FreezeRequest,
        key: &JobKey,
        target: BackupTargetKind,
        paths: OwnerExportPaths,
    ) -> Result<FrozenOwners> {
        self.freeze_timed(request, key, target, paths, &mut StepTimes::start())
    }
    /// Intent 必须已由独立作业方落账；整个窗口共用一个原截止时间，任何失败按逆序释放。
    /// 走过的每一步记进 `steps`，失败时已走过的照样留着，生产者照它写日志（BUG-050）。
    pub fn freeze_timed(
        &self,
        request: FreezeRequest,
        key: &JobKey,
        target: BackupTargetKind,
        paths: OwnerExportPaths,
        steps: &mut StepTimes,
    ) -> Result<FrozenOwners> {
        if request.checkpoint_id != key.checkpoint_id {
            return Err(unavailable(Owner::Audit));
        }
        let intent = backup_intent(key, target).map_err(|_| unavailable(Owner::Audit))?;
        let deadline = Instant::now() + Duration::from_millis(u64::from(request.budget.millis()));
        // 登录库自己补得掉的待办先补完：补记要取完整操作许可，所以只能在排空之前做。
        self.settle_auth(deadline)?;
        steps.mark("补记登录库");
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
        steps.mark("排空接入方");
        let human_quiet = self
            .audit
            .quiesce(deadline)
            .map_err(|_| unavailable(Owner::Audit))?;
        steps.mark("排空人侧");
        check_deadline(deadline, Owner::Broker)?;
        let broker = self
            .workbench
            .freeze_checkpoint(deadline)
            .map_err(|_| unavailable(Owner::Broker))?;
        steps.mark("写者冻结");
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
        // 控制、登录、Bridge、接入方、策略和两本账本，没装的也照样记这一步，日志形状固定。
        steps.mark("各库冻结");
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
            checked: None,
            deadline,
        };
        frozen.pin(steps)?;
        Ok(frozen)
    }
}
impl FrozenOwners {
    /// 各库都冻住之后：核本作业的 Intent，定格写者的两棵树，核 Bridge 的持久 ACK 与传输最后一次运行
    /// （Bridge 整份在窗口里导出，见 [`FrozenOwners::release`]）。
    /// 写者的重放与回执随编辑数涨，窗口里不读，放开之后在导出的副本上核（BUG-050）；
    /// 登录、控制、接入方库的回执只读各自定格的库，也在放开之后核（[`ReleasedOwners`]）。
    fn pin(&mut self, steps: &mut StepTimes) -> Result<()> {
        self.checked = None;
        self.audit
            .verify_event(&self.intent)
            .map_err(|_| unavailable(Owner::Audit))?;
        steps.mark("核 Intent");
        // 只有实际核验成功才累计，包括本作业在副作用前已落账的 Intent。
        let mut checked = 1_u64;
        // 写者自己分得清「超过固定上限」和「不可用」，原样带上（BUG-045）。
        self.broker.capture()?;
        steps.mark("定格写者两棵树");
        if let Some(bridge) = &mut self.bridge {
            add(
                &mut checked,
                bridge
                    .verify_audit(&self.audit)
                    .map_err(|_| unavailable(Owner::Bridge))?,
            )?;
        }
        steps.mark("核 Bridge 回执");
        check_deadline(self.deadline, Owner::Audit)?;
        self.checked = Some(checked);
        Ok(())
    }
    /// 不记分步的放开，只给用例用；生产封存用 [`Self::release_timed`]。
    #[cfg(test)]
    pub(crate) fn release(self, deadline: Instant) -> Result<ReleasedOwners> {
        self.release_timed(deadline, &mut StepTimes::start())
    }
    /// 窗口里收尾，再按冻结的逆序放开全部屏障（BUG-045）。
    ///
    /// 窗口里只做两件要读活对象、又不随数据涨的事：交出写者的定格（两棵树的文件句柄），
    /// Bridge 库连同传输、平面文件整份导出——Bridge 每次写前写后都要求 WAL 截断干净、截不掉就封掉实例，
    /// 放开之后不能再留着它的定格读事务；它的库有页数上限，和传输、平面文件一样是固定大小的状态。
    /// 然后放开两本账本、策略、接入方库的写锁，放开 Bridge、登录、控制库（定格的读连接留着），
    /// 把写者还回工作台，最后恢复人侧和接入方的完整操作准入。之后的活都受 `deadline` 约束。
    /// 任何一步失败，剩下的屏障随返回值析构照样按逆序放开。
    /// 走过的每一步接着记进冻结时那一份 `steps`（BUG-050）。
    pub fn release_timed(
        mut self,
        deadline: Instant,
        steps: &mut StepTimes,
    ) -> Result<ReleasedOwners> {
        // 全程在 self 上做：中途失败时整个结构按字段声明顺序析构，先放库屏障、最后才恢复准入。
        let checked = self.checked.ok_or_else(|| unavailable(Owner::Audit))?;
        let capture = self
            .broker
            .take_capture()
            .map_err(|_| unavailable(Owner::Broker))?;
        steps.mark("交出写者定格");
        let bridged = self
            .bridge
            .as_ref()
            .map(|bridge| {
                bridge
                    .export(&self.paths.bridge, &self.paths.transport, &self.paths.plane)
                    .map_err(|_| unavailable(Owner::Bridge))
            })
            .transpose()?;
        steps.mark("Bridge 导出");
        check_deadline(self.deadline, Owner::Audit)?;
        self.audit
            .release(deadline)
            .map_err(|_| unavailable(Owner::Audit))?;
        if let Some(secondary) = &mut self.agent_audit {
            secondary
                .release(deadline)
                .map_err(|_| unavailable(Owner::Audit))?;
        }
        if let Some(participant) = &self.policy {
            participant.release(deadline)?;
        }
        if let Some(store) = &mut self.agents {
            store
                .release(deadline)
                .map_err(|_| unavailable(Owner::AgentStore))?;
        }
        drop(self.bridge.take());
        self.auth
            .release(deadline)
            .map_err(|_| unavailable(Owner::Auth))?;
        if let Some(board) = &mut self.control {
            board
                .release(deadline)
                .map_err(|_| unavailable(Owner::Control))?;
        }
        // 之后没有会失败的步骤：拆开，写者还回工作台，最后恢复人侧和接入方的完整操作准入。
        let FrozenOwners {
            audit,
            agent_audit,
            policy,
            agents,
            bridge: _,
            auth,
            control,
            broker,
            _human_quiet: human_quiet,
            _agent_quiet: agent_quiet,
            request,
            paths,
            ..
        } = self;
        drop(broker);
        drop(human_quiet);
        drop(agent_quiet);
        steps.mark("放开");
        Ok(ReleasedOwners {
            audit,
            agent_audit,
            policy,
            agents,
            bridge: bridged,
            auth,
            control,
            broker: capture,
            request,
            paths,
            checked,
            deadline,
        })
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
