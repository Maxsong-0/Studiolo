//! 把 `SyncTransportOwner` 的 `QuiescentMirror` 适配成执行器的 `MirrorQuiescence`。
//!
//! 唯一被接受的证据是「同一受控子组 populated=0 且 spawn 入口已关闭」，它就是
//! `SyncTransportOwner::quiescent` 成功返回的含义；本文件不自造证据，每次 `observe`
//! 前都先 `verify_quiescent`，票据被作废即返回 `Unavailable`。

use super::{PlaneError, SyncPlane};
use std::time::Instant;
use studiolo_sync_bridge::executor::{
    MirrorQuiescence, MirrorQuiescenceProof, QuiescenceFault, QuiescenceRequest,
};
use studiolo_sync_transport::owned_run::QuiescentMirror;

/// 一张静止票据加它的绑定事实与到期时刻。
pub struct OwnerQuiescence<'a> {
    plane: &'a SyncPlane,
    ticket: QuiescentMirror,
    link_id: [u8; 16],
    scope_epoch: u64,
    mirror_root_token: [u8; 32],
    mirror_observation: u64,
    expires_at: Instant,
}

impl<'a> OwnerQuiescence<'a> {
    /// 从传输槽取当前静止票据；`mirror_observation` 是本轮镜像观察序号。
    ///
    /// # Errors
    /// 没有静止证明或票据已作废返回 `Quiescence`。
    pub fn take(
        plane: &'a SyncPlane,
        link_id: [u8; 16],
        scope_epoch: u64,
        mirror_root_token: [u8; 32],
        mirror_observation: u64,
        now: Instant,
    ) -> Result<Self, PlaneError> {
        // B 轮 W3 实现：plane.transport.lock() → quiescent() → 记 expires_at = now + QUIESCENT_WINDOW。
        let _ = (
            plane,
            link_id,
            scope_epoch,
            mirror_root_token,
            mirror_observation,
            now,
        );
        Err(PlaneError::Quiescence)
    }

    /// 票据本体；镜像安装前用它复验视图根身份。
    #[must_use]
    pub fn ticket(&self) -> &QuiescentMirror {
        &self.ticket
    }

    /// 所属平面。
    #[must_use]
    pub fn plane(&self) -> &'a SyncPlane {
        self.plane
    }

    /// 到期时刻。
    #[must_use]
    pub fn expires_at(&self) -> Instant {
        self.expires_at
    }
}

impl MirrorQuiescence for OwnerQuiescence<'_> {
    fn observe(
        &self,
        request: &QuiescenceRequest,
    ) -> Result<MirrorQuiescenceProof, QuiescenceFault> {
        // B 轮 W3 实现：verify_quiescent 失败 → Unavailable；绑定不符 → BindingMismatch；
        // 通过则 evidence = ControlledSubgroupIdle { spawn_gate_closed: true }。
        let _ = (
            request,
            self.link_id,
            self.scope_epoch,
            self.mirror_root_token,
            self.mirror_observation,
        );
        Err(QuiescenceFault::Unavailable)
    }
}
