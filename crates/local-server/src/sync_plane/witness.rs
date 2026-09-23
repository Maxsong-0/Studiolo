//! 可信观察 adapter：canonical 完整观察经 `coordinator::sync_bridge` 接缝取得，
//! 镜像观察经 `mirror::observe`；两者合成执行器要的 `ExecutionWitness`。
//!
//! 本文件不持有 Broker 类型、不开文件。

use super::{PlaneError, SyncPlane};
use studiolo_sync_bridge::executor::{
    CommonRecheck, ExecutionRecheck, ExecutionWitness, Quiesced, WitnessFault,
};
use studiolo_sync_bridge::{ActionKey, ApprovedScope, Binding, Inventory};

/// 一次执行期间的观察上下文；`epoch` 是本次打开实例的随机标识，跨重开不复用。
pub struct PlaneWitness<'a> {
    plane: &'a SyncPlane,
    binding: Binding,
    approved: ApprovedScope,
    epoch: [u8; 32],
    next_canonical: u64,
    next_mirror: u64,
}

impl<'a> PlaneWitness<'a> {
    /// 从冻结绑定与已核准范围构造；观察序号从游标 +1 起。
    #[must_use]
    pub fn new(
        plane: &'a SyncPlane,
        binding: Binding,
        approved: ApprovedScope,
        epoch: [u8; 32],
        next_canonical: u64,
        next_mirror: u64,
    ) -> Self {
        Self {
            plane,
            binding,
            approved,
            epoch,
            next_canonical,
            next_mirror,
        }
    }

    /// canonical 侧完整观察：经接缝取 `CanonicalSyncObservation`，逐条换成
    /// `Presence`（revision＝Broker revision 字节，sha256／bytes 取自 BlobIdentity）。
    ///
    /// # Errors
    /// 观察不完整或范围外返回 `Observation`；写者不可用返回 `Unavailable`。
    pub fn observe_canonical(&mut self) -> Result<Inventory, PlaneError> {
        // B 轮 W3 实现：selection 由 scope.observation_selection() 投影成 SyncReadSelection；
        // required = approved.paths；complete=true 只在接缝没有诊断时。
        Err(PlaneError::Observation)
    }

    /// 本次实例的冻结绑定。
    #[must_use]
    pub fn binding(&self) -> &Binding {
        &self.binding
    }

    /// 已核准范围。
    #[must_use]
    pub fn approved(&self) -> &ApprovedScope {
        &self.approved
    }

    /// 所属平面。
    #[must_use]
    pub fn plane(&self) -> &'a SyncPlane {
        self.plane
    }

    /// 打开实例标识。
    #[must_use]
    pub fn epoch(&self) -> [u8; 32] {
        self.epoch
    }

    /// 下一对观察序号 `(canonical, mirror)`。
    #[must_use]
    pub fn next_sequences(&self) -> (u64, u64) {
        (self.next_canonical, self.next_mirror)
    }
}

impl ExecutionWitness for PlaneWitness<'_> {
    fn recheck(
        &mut self,
        key: &ActionKey,
        quiesced: &Quiesced,
    ) -> Result<ExecutionRecheck, WitnessFault> {
        // B 轮 W3 实现：源侧与目标侧各做一次单路径新鲜观察，expires_at = quiesced.deadline()。
        let _ = (key, quiesced);
        Err(WitnessFault::Unavailable)
    }

    fn common(&mut self, quiesced: &Quiesced) -> Result<Option<CommonRecheck>, WitnessFault> {
        // B 轮 W3 实现：两侧完整重观察；任一侧不完整返回 Ok(None)，不得补造。
        let _ = quiesced;
        Err(WitnessFault::Unavailable)
    }
}
