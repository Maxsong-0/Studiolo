//! 实际平面接线：移走唯一传输 owner、持有 plane.lock，Bridge 每次签封都复验同一能力。
use super::{PlaneError, SyncPlane, bridge_binding, recovery::FrozenPlaneFiles};
use crate::sync_transport_owner::transport_event;
use std::{
    path::Path,
    sync::{Arc, Mutex},
    time::Instant,
};
use studiolo_audit_ledger::FrozenAudit;
use studiolo_core::checkpoint::{OwnerWatermark, SealedOwnerCheckpoint};
use studiolo_sync_bridge::{
    Binding,
    sqlite::{BridgeCheckpointGate, BridgeError, FrozenBridge},
};
use studiolo_sync_transport::owned_run::{FrozenTransport, TransportError};

type TransportSlot = Arc<Mutex<Option<FrozenTransport<'static>>>>;

/// 三类辅助文件与原 Bridge 库共同持有；字段析构顺序保证先释放库屏障，最后返还原传输实例。
pub struct FrozenSyncPlane {
    bridge: FrozenBridge,
    files: Arc<Mutex<FrozenPlaneFiles>>,
    restore: RestoreTransport,
}
struct RestoreTransport {
    plane: Arc<SyncPlane>,
    frozen: TransportSlot,
}
impl Drop for RestoreTransport {
    fn drop(&mut self) {
        // 冻结期间不执行普通操作；即使上层展开 panic，也返还同一个 owner 和原 lease。
        let mut frozen = self
            .frozen
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut slot = self
            .plane
            .transport
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if slot.is_none() {
            *slot = frozen.take().map(FrozenTransport::thaw);
        }
    }
}
struct PlaneGate {
    plane: Arc<SyncPlane>,
    frozen: TransportSlot,
    files: Arc<Mutex<FrozenPlaneFiles>>,
    deadline: Instant,
}
impl BridgeCheckpointGate for PlaneGate {
    fn verify(&self, binding: &Binding, deadline: Instant) -> Result<(), BridgeError> {
        if Instant::now() >= self.deadline.min(deadline) {
            return Err(BridgeError::Unavailable);
        }
        self.plane
            .approval
            .revalidate()
            .map_err(|_| BridgeError::BindingMismatch)?;
        self.plane
            .workbench
            .core
            .roots
            .revalidate()
            .map_err(|_| BridgeError::BindingMismatch)?;
        let expected = bridge_binding(
            self.plane.approval,
            self.plane.workbench.core.roots.vault_identity(),
        );
        if binding != &expected {
            return Err(BridgeError::BindingMismatch);
        }
        let frozen = self
            .frozen
            .try_lock()
            .map_err(|_| BridgeError::Unavailable)?;
        let actual = frozen
            .as_ref()
            .ok_or(BridgeError::Unavailable)?
            .binding()
            .map_err(|_| BridgeError::Unavailable)?;
        let approved =
            studiolo_sync_transport::owned_run::ApprovedTransport::binding(self.plane.approval)
                .map_err(|_| BridgeError::BindingMismatch)?;
        if actual != approved {
            return Err(BridgeError::BindingMismatch);
        }
        self.files
            .try_lock()
            .map_err(|_| BridgeError::Unavailable)?
            .verify()
            .map_err(|_| BridgeError::Unavailable)?;
        if Instant::now() >= self.deadline.min(deadline) {
            return Err(BridgeError::Unavailable);
        }
        Ok(())
    }
}
impl SyncPlane {
    /// 全局操作许可排空后，取得真正传输和附属存储门禁，再冻结原 Bridge 数据库。
    /// 任一步失败都返还原实例；不启动新运行，不隐式清理失败现场。
    pub fn freeze_checkpoint(
        self: &Arc<Self>,
        deadline: Instant,
    ) -> Result<FrozenSyncPlane, PlaneError> {
        let transport = self
            .transport
            .try_lock()
            .map_err(|_| PlaneError::Unavailable)?
            .take()
            .ok_or(PlaneError::Unavailable)?;
        let restore = RestoreTransport {
            plane: self.clone(),
            frozen: Arc::new(Mutex::new(Some(transport.freeze_checkpoint(deadline)))),
        };
        let files = Arc::new(Mutex::new(FrozenPlaneFiles::open(
            &self.plane_root,
            deadline,
        )?));
        let gate = Arc::new(PlaneGate {
            plane: self.clone(),
            frozen: restore.frozen.clone(),
            files: files.clone(),
            deadline,
        });
        let source = self
            .bridge
            .try_lock()
            .map_err(|_| PlaneError::Unavailable)?
            .checkpoint_source()
            .map_err(|_| PlaneError::Storage)?;
        let bridge = source
            .freeze(gate, deadline)
            .map_err(|_| PlaneError::Storage)?;
        Ok(FrozenSyncPlane {
            bridge,
            files,
            restore,
        })
    }
}
impl FrozenSyncPlane {
    /// 数据库共同基线水位；辅助导出成功后才可作为同一个 Bridge owner 签封。
    pub fn watermark(&self) -> &OwnerWatermark {
        self.bridge.watermark()
    }
    /// Bridge 持久 ACK 与 Transport 最后运行原事件，全部核对同一真实人侧 FrozenAudit。
    pub fn verify_audit(&mut self, audit: &FrozenAudit) -> Result<(), PlaneError> {
        self.bridge
            .verify_audit(audit)
            .map_err(|_| PlaneError::Audit)?;
        self.restore
            .frozen
            .try_lock()
            .map_err(|_| PlaneError::Unavailable)?
            .as_mut()
            .ok_or(PlaneError::Unavailable)?
            .verify_audit(|candidate| {
                let event =
                    transport_event(candidate.event_id(), candidate.phase(), candidate.payload())
                        .map_err(|_| TransportError::AuditRejected)?;
                audit
                    .verify_event(&event)
                    .map(|_| ())
                    .map_err(|_| TransportError::AuditRejected)
            })
            .map_err(|_| PlaneError::Audit)
    }
    /// 三个独立空目标由最终 inclusion plan 分配；全部完成才返回 Bridge 的签封回执。
    pub fn export(
        &self,
        database: &Path,
        transport: &Path,
        plane: &Path,
    ) -> Result<SealedOwnerCheckpoint, PlaneError> {
        let sealed = self
            .bridge
            .export(database)
            .map_err(|_| PlaneError::Storage)?;
        self.restore
            .frozen
            .try_lock()
            .map_err(|_| PlaneError::Unavailable)?
            .as_ref()
            .ok_or(PlaneError::Unavailable)?
            .export(transport)
            .map_err(|_| PlaneError::Storage)?;
        self.files
            .try_lock()
            .map_err(|_| PlaneError::Unavailable)?
            .export(plane)?;
        // 辅助文件复制期间也不能让原绑定或截止时间漂移；此调用仅验源，不重写目标。
        self.bridge
            .verify_source()
            .map_err(|_| PlaneError::Storage)?;
        Ok(sealed)
    }
}
