//! 最小封闭 API：固定 link、一次一个运行的 `SyncTransportOwner`。
//!
//! 它只能由已核验的受控 Linux 后端打开。当前仓库还拿不到那个后端，因此本类型的
//! 生命周期方法在真实平台验收之前不可达；这是如实的平台状态，不是占位实现。

use super::approval::{ApprovedTransport, PinnedRuntime};
use super::audit::RunAuditCandidate;
use super::error::{Result, TransportError};
use super::session::{RunSession, random};
use super::state::{OwnerIdentity, SCHEMA_VERSION, TransportState, encode};
use super::tickets::{OwnedRun, PreparedRun, QuiescentMirror};
use super::types::{RunId, RunObservation, RunOperation, RunStatus, StopReason};
use crate::linux::{LinuxContainment, VerifiedLinuxBackend};
use crate::store::{FilePublisher, StatePublisher};

/// 固定 link 的一次一运行 owner。
pub struct SyncTransportOwner<'scope> {
    inner: RunSession<'scope, LinuxContainment>,
}

impl<'scope> SyncTransportOwner<'scope> {
    /// 打开既有槽；不补目录、不补空 state、不认领残留。
    ///
    /// # Errors
    /// 受控隔离前置未满足时不会走到这里（`VerifiedLinuxBackend` 拿不到）；
    /// 存储缺失返回 `MissingStore`，绑定不符返回 `BindingMismatch`。
    pub fn open_existing(
        approval: &'scope dyn ApprovedTransport,
        runtime: PinnedRuntime,
        backend: VerifiedLinuxBackend,
    ) -> Result<Self> {
        let publisher = FilePublisher::open_existing(approval.roots().private_root.as_path())?;
        Ok(Self {
            inner: RunSession::open(
                approval,
                runtime,
                LinuxContainment::new(backend),
                Box::new(publisher),
            )?,
        })
    }

    /// 安全逻辑元数据。
    ///
    /// # Errors
    /// 配置或根漂移、记录损坏时返回对应固定分类。
    pub fn inspect(&self) -> Result<RunStatus> {
        self.inner.inspect()
    }

    /// 准备一次运行；Intent 必须先 ACK 才能启动。
    ///
    /// # Errors
    /// 活动槽未空返回 `RunActive`；序号不符返回 `SequenceMismatch`。
    pub fn prepare_once(
        &mut self,
        expected_sequence: u64,
        operation: RunOperation,
        requested_at_millis: u64,
    ) -> Result<PreparedRun> {
        self.inner
            .prepare_once(expected_sequence, operation, requested_at_millis)
    }

    /// 当前待补审计候选。
    ///
    /// # Errors
    /// 记录损坏或配置漂移时返回对应固定分类。
    pub fn pending_audit(&self) -> Result<Option<RunAuditCandidate>> {
        self.inner.pending_audit()
    }

    /// 登记 owner 已核验的真实 AuditRecord 回执。
    ///
    /// # Errors
    /// 与待补事件不符返回 `AuditMismatch`；偏移为零或哈希全零返回 `AuditRejected`。
    pub fn confirm_audit(
        &mut self,
        candidate: &RunAuditCandidate,
        offset: u64,
        hash: [u8; 32],
    ) -> Result<()> {
        self.inner.confirm_audit(candidate, offset, hash)
    }

    /// 放行一次执行；重复请求不得产生第二次 spawn。
    ///
    /// # Errors
    /// 票据已用返回 `AlreadyStarted`；启动后失败返回 `ExitUnconfirmed`。
    pub fn start(&mut self, prepared: &PreparedRun) -> Result<OwnedRun> {
        self.inner.start(prepared)
    }

    /// 仅尚未 LaunchIssued 时取消；固定「未执行」Outcome。
    ///
    /// # Errors
    /// 已发出启动请求返回 `PhaseConflict`。
    pub fn cancel_prepared(&mut self, prepared: PreparedRun) -> Result<()> {
        self.inner.cancel_prepared(prepared)
    }

    /// 停止原因先持久，再只停绑定子组。
    ///
    /// # Errors
    /// 阶段不允许返回 `PhaseConflict`。
    pub fn request_stop(&mut self, owned: &OwnedRun, reason: StopReason) -> Result<()> {
        self.inner.request_stop(owned, reason)
    }

    /// 推进到当前可证明的观察。
    ///
    /// # Errors
    /// 启动后失去容器身份返回 `ExitUnconfirmed`。
    pub fn wait(&mut self, owned: &OwnedRun) -> Result<RunObservation> {
        self.inner.wait(owned)
    }

    /// 只观察旧容器，不重新执行。
    ///
    /// # Errors
    /// 没有对应活动运行返回 `NotFound`。
    pub fn recover(&mut self, run_id: RunId) -> Result<RunObservation> {
        self.inner.recover(run_id)
    }

    /// 只在完整匹配旧容器时停止。
    ///
    /// # Errors
    /// 身份不符返回 `ExitUnconfirmed`。
    pub fn stop_recovered(&mut self, run_id: RunId) -> Result<()> {
        self.inner.stop_recovered(run_id)
    }

    /// 已有静止证据且 Outcome 已 ACK 才发；本地静止不代表远端静止。
    ///
    /// # Errors
    /// 没有静止证明返回 `QuiescenceUnconfirmed`。
    pub fn quiescent(&self) -> Result<QuiescentMirror> {
        self.inner.quiescent()
    }

    /// 复验静止能力；新一轮 prepare、根变化、租约变化与 owner 重开都会作废它。
    ///
    /// # Errors
    /// 跨实例返回 `TicketForeign`；已被作废返回 `TicketStale`。
    pub fn verify_quiescent(&self, mirror: &QuiescentMirror) -> Result<()> {
        self.inner.verify_quiescent(mirror)
    }
}

/// 构造首条完整记录；初始化是管理员显式动作，启动流程不得隐式创建它。
///
/// # Errors
/// 绑定代次为零返回 `BindingMismatch`；随机源不可用返回 `Unavailable`。
pub fn initial_state(
    approval: &dyn ApprovedTransport,
    runtime: &PinnedRuntime,
    owner: OwnerIdentity,
) -> Result<TransportState> {
    approval.revalidate()?;
    let mut installation_id = [0u8; 16];
    random(&mut installation_id)?;
    let state = TransportState {
        schema_version: SCHEMA_VERSION,
        installation_id,
        generation: 1,
        next_sequence: 1,
        binding: approval.binding()?,
        roots: approval.roots(),
        runtime: runtime.pins().clone(),
        backend: approval.backend(),
        owner,
        active: None,
        last_terminal: None,
    };
    super::state::validate(&state)?;
    Ok(state)
}

/// 管理员显式初始化传输私有存储；三个固定名字都不存在时才建库。
///
/// # Errors
/// 已存在返回 `AlreadyExists`；根不是 0700 私有目录返回 `UnsafeStorage`。
pub fn initialize_store(approval: &dyn ApprovedTransport, runtime: &PinnedRuntime) -> Result<()> {
    let root = approval.roots().private_root;
    let (uid, gid) = current_identity()?;
    let state = initial_state(approval, runtime, OwnerIdentity { uid, gid })?;
    let publisher = FilePublisher::initialize(root.as_path(), &encode(&state)?)?;
    if publisher.owner_identity() != (uid, gid) {
        return Err(TransportError::UnsafeStorage);
    }
    Ok(())
}

#[cfg(unix)]
fn current_identity() -> Result<(u32, u32)> {
    use rustix::process::{getegid, geteuid, getgid, getuid};
    if getuid() != geteuid() || getgid() != getegid() {
        return Err(TransportError::UnsafeStorage);
    }
    Ok((geteuid().as_raw(), getegid().as_raw()))
}
