//! 显式维护重绑定：只重写已静止的既有槽，不恢复旧运行时、不初始化、不迁移实体。

use super::approval::{ApprovedTransport, PinnedRuntime};
use super::state::{OwnerIdentity, TransportState, decode, encode, validate};
use super::{Quiescence, Result, RunPhase, TransportError};
use crate::store::{FilePublisher, StatePublisher};
use serde::Serialize;

/// 主人确认只展示非秘密摘要与代次，不展示配置正文、路径或运行输出。
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RebindReview {
    /// 原安全配置摘要。
    pub previous_config_digest: [u8; 32],
    /// 新安全配置摘要。
    pub next_config_digest: [u8; 32],
    /// 原提交代次。
    pub previous_generation: u64,
    /// 将发布的提交代次。
    pub next_generation: u64,
}

/// 持有既有 transport.lock 的维护会话；与正常 owner 相互排斥。
pub struct TransportRebind<'scope> {
    approval: &'scope dyn ApprovedTransport,
    runtime: PinnedRuntime,
    publisher: Box<dyn StatePublisher>,
    previous: TransportState,
    next: TransportState,
}

impl<'scope> TransportRebind<'scope> {
    /// 在新审批路径打开既有槽；旧路径或旧软件不可读并不妨碍读取持久事实。
    ///
    /// # 错误
    /// 活动或审计未闭合拒绝；范围、owner 或物理私有根/镜像变更拒绝。
    pub fn open_existing(approval: &'scope dyn ApprovedTransport) -> Result<Self> {
        approval.revalidate()?;
        let publisher = FilePublisher::open_existing(&approval.roots().private_root)?;
        Self::open(approval, Box::new(publisher))
    }

    pub(crate) fn open(
        approval: &'scope dyn ApprovedTransport,
        publisher: Box<dyn StatePublisher>,
    ) -> Result<Self> {
        approval.revalidate()?;
        publisher.check()?;
        let runtime = PinnedRuntime::from_approval(approval)?;
        let previous = decode(&publisher.read()?)?;
        let (uid, gid) = publisher.owner_identity();
        let next = transition(&previous, approval, &runtime, OwnerIdentity { uid, gid })?;
        let session = Self {
            approval,
            runtime,
            publisher,
            previous,
            next,
        };
        session.revalidate()?;
        Ok(session)
    }

    /// 供确认界面使用的固定元数据。
    #[must_use]
    pub fn review(&self) -> RebindReview {
        RebindReview {
            previous_config_digest: self.previous.binding.approved_config_digest,
            next_config_digest: self.next.binding.approved_config_digest,
            previous_generation: self.previous.generation,
            next_generation: self.next.generation,
        }
    }

    /// 复验审批、新运行时 pins、租约及未写入的原状态；不给旧批准开例外。
    ///
    /// # 错误
    /// 任何快照、根或代次漂移均拒绝。
    pub fn revalidate(&self) -> Result<()> {
        self.approval.revalidate()?;
        self.publisher.check()?;
        if PinnedRuntime::from_approval(self.approval)? != self.runtime {
            return Err(TransportError::RuntimeChanged);
        }
        let current = decode(&self.publisher.read()?)?;
        if current != self.previous {
            return Err(TransportError::GenerationConflict);
        }
        let (uid, gid) = self.publisher.owner_identity();
        if transition(
            &current,
            self.approval,
            &self.runtime,
            OwnerIdentity { uid, gid },
        )? != self.next
        {
            return Err(TransportError::ConfigurationChanged);
        }
        self.publisher.check()?;
        self.approval.revalidate()
    }

    /// 所有前置成功且主人已确认后，完整原子替换一次；不重新执行旧运行。
    ///
    /// # 错误
    /// 写前漂移拒绝；发布失败沿用持久层固定分类，调用方必须重开核验结果。
    pub fn commit(mut self) -> Result<RebindReview> {
        self.revalidate()?;
        let review = self.review();
        self.publisher.replace(&encode(&self.next)?)?;
        self.publisher.check()?;
        self.approval.revalidate()?;
        if PinnedRuntime::from_approval(self.approval)? != self.runtime {
            return Err(TransportError::RuntimeChanged);
        }
        if self.approval.binding()? != self.next.binding
            || self.approval.backend() != self.next.backend
        {
            return Err(TransportError::ConfigurationChanged);
        }
        if self.approval.roots() != self.next.roots {
            return Err(TransportError::RootsChanged);
        }
        if decode(&self.publisher.read()?)? != self.next {
            return Err(TransportError::GenerationConflict);
        }
        self.publisher.check()?;
        self.approval.revalidate()?;
        Ok(review)
    }
}

/// 纯状态变换：无 IO、无旧运行时要求；调用方以实际租约 owner 与新批准事实调用。
pub(crate) fn transition(
    previous: &TransportState,
    approval: &dyn ApprovedTransport,
    runtime: &PinnedRuntime,
    owner: OwnerIdentity,
) -> Result<TransportState> {
    validate(previous)?;
    if previous.owner != owner {
        return Err(TransportError::UnsafeStorage);
    }
    if let Some(active) = &previous.active {
        return Err(if active.audit_pending() {
            TransportError::AuditPending
        } else {
            TransportError::RunActive
        });
    }
    if previous.last_terminal.as_ref().is_some_and(|terminal| {
        terminal.observation.phase != RunPhase::Closed
            || terminal.observation.quiescence != Quiescence::Confirmed
    }) {
        return Err(TransportError::QuiescenceUnconfirmed);
    }
    let binding = approval.binding()?;
    let mut old_binding = previous.binding.clone();
    old_binding.approved_config_digest = binding.approved_config_digest;
    if old_binding != binding {
        return Err(TransportError::BindingMismatch);
    }
    let roots = approval.roots();
    if roots.private_identity != previous.roots.private_identity
        || roots.mirror_identity != previous.roots.mirror_identity
    {
        return Err(TransportError::RootsChanged);
    }
    if !approval.budget().complete() {
        return Err(TransportError::Budget);
    }
    if approval.runtime() != *runtime.pins() {
        return Err(TransportError::RuntimeChanged);
    }
    let next = TransportState {
        generation: previous
            .generation
            .checked_add(1)
            .ok_or(TransportError::CounterOverflow)?,
        binding,
        roots,
        runtime: runtime.pins().clone(),
        backend: approval.backend(),
        // 旧静止事实不能给新配置发票；安装身份与已消耗序号必须保留。
        last_terminal: None,
        ..previous.clone()
    };
    validate(&next)?;
    Ok(next)
}
