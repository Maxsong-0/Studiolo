//! 把 `SyncTransportOwner` 的 `QuiescentMirror` 适配成执行器的 `MirrorQuiescence`。
//!
//! 唯一被接受的证据是「同一受控子组 populated=0 且 spawn 入口已关闭」，它就是
//! `SyncTransportOwner::quiescent` 成功返回的含义；本文件不自造证据，每次 `observe`
//! 前都先 `verify_quiescent`，票据被作废即返回 `Unavailable`。平面自己发起的镜像读写
//! （观察、入站搬运读、安装、核对、冲突保全、排除项守卫）都经 [`verify`] 复验同一张票据；
//! 执行器批内对镜像侧 Delete 的删除前保全读由批首取票与逐动作 recheck 覆盖，不单独复验。

use super::mirror::MirrorView;
use super::{PlaneError, QUIESCENT_WINDOW, SyncPlane};
use std::time::Instant;
use studiolo_sync_bridge::executor::{
    MirrorQuiescence, MirrorQuiescenceProof, QuiescenceEvidence, QuiescenceFault, QuiescenceRequest,
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
    /// 从传输槽取当前静止票据；`mirror_observation` 是本轮在静止窗口内取得的镜像观察序号。
    ///
    /// # Errors
    /// 没有静止证明或票据已作废返回 `Quiescence`；传输槽锁不可用返回 `Unavailable`。
    pub fn take(
        plane: &'a SyncPlane,
        link_id: [u8; 16],
        scope_epoch: u64,
        mirror_root_token: [u8; 32],
        mirror_observation: u64,
        now: Instant,
    ) -> Result<Self, PlaneError> {
        let ticket = plane
            .transport()
            .lock()
            .map_err(|_| PlaneError::Unavailable)?
            .quiescent()
            .map_err(|_| PlaneError::Quiescence)?;
        Ok(Self {
            plane,
            ticket,
            link_id,
            scope_epoch,
            mirror_root_token,
            mirror_observation,
            expires_at: now + QUIESCENT_WINDOW,
        })
    }

    /// 票据本体；镜像安装前用它复验视图根身份。
    #[must_use]
    pub fn ticket(&self) -> &QuiescentMirror {
        &self.ticket
    }

    /// 票据证明静止的视图根与取证时刻的身份。
    #[must_use]
    pub fn view(&self) -> MirrorView {
        MirrorView::new(
            self.ticket.mirror_root().to_path_buf(),
            self.ticket.mirror_identity(),
        )
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

/// 每次读写镜像前的固定复验：第 7 步的票据仍有效才继续。
///
/// # Errors
/// 传输槽锁不可用返回 `Unavailable`；票据已作废（新一轮 prepare、根或租约变化、owner 重开）返回 `Quiescence`。
pub fn verify(plane: &SyncPlane, ticket: &QuiescentMirror) -> Result<(), PlaneError> {
    plane
        .transport()
        .lock()
        .map_err(|_| PlaneError::Unavailable)?
        .verify_quiescent(ticket)
        .map_err(|_| PlaneError::Quiescence)
}

impl MirrorQuiescence for OwnerQuiescence<'_> {
    fn observe(
        &self,
        request: &QuiescenceRequest,
    ) -> Result<MirrorQuiescenceProof, QuiescenceFault> {
        self.plane
            .transport()
            .lock()
            .map_err(|_| QuiescenceFault::Unavailable)?
            .verify_quiescent(&self.ticket)
            .map_err(|_| QuiescenceFault::Unavailable)?;
        if request.link_id != self.link_id
            || request.scope_epoch != self.scope_epoch
            || request.mirror_root_token != self.mirror_root_token
        {
            return Err(QuiescenceFault::BindingMismatch);
        }
        if self.mirror_observation < request.minimum_mirror_observation {
            return Err(QuiescenceFault::Stale);
        }
        Ok(MirrorQuiescenceProof {
            link_id: self.link_id,
            scope_epoch: self.scope_epoch,
            mirror_root_token: self.mirror_root_token,
            evidence: QuiescenceEvidence::ControlledSubgroupIdle {
                spawn_gate_closed: true,
            },
            mirror_observation: self.mirror_observation,
            expires_at: self.expires_at,
        })
    }
}
