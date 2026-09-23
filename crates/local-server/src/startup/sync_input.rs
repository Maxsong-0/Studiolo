//! 双向 Bridge 平面的组合根（ADR-025 D1）：只在显式 `--sync-transport` 且私有配置带
//! `sync_scope` 时装配；缺任一项即 `Ok(None)`，合入 main 不改变生产行为。
//!
//! 装配顺序：审批文档 → 借用范围追加审批（根 pins）→ 固定运行时 → 经 launcher 探测受控后端
//! → 打开传输运行槽 → 打开 Bridge 持久状态 → 组装平面。任一步失败返回
//! `StartupError::SyncPlane`，不监听。审批与范围经 `Box::leak` 取得进程生命期引用：
//! 平面与进程同生死，这两份只读能力也是。

use super::StartupError;
use crate::options::Options;
use sha2::{Digest, Sha256};
use std::{path::Path, sync::Arc};
use studiolo_audit_ledger::AuditLedger;
use studiolo_host_config::SyncScope;
use studiolo_host_config::sync_transport::{ApprovedTransport, SyncTransportApproval};
use studiolo_local_server::CanonicalWorkbench;
use studiolo_local_server::sync_plane::{PlaneInputs, SyncPlane};
use studiolo_sync_bridge::{Binding, sqlite::BridgeState};
use studiolo_sync_transport::linux::verify_backend;
use studiolo_sync_transport::owned_run::{PinnedRuntime, SyncTransportOwner};

/// `<plane_root>` 下 Bridge 持久状态的固定子目录。
pub(super) const BRIDGE_DIR: &str = "bridge";

/// 装配平面；默认关闭路径在这里就返回 `Ok(None)`。
pub(super) fn assemble(
    options: &Options,
    scope: Option<SyncScope>,
    canonical_identity: Option<(u64, u64)>,
    workbench: Option<&Arc<CanonicalWorkbench>>,
    audit: Option<&Arc<dyn AuditLedger>>,
    reserved: &[std::path::PathBuf],
) -> Result<Option<Arc<SyncPlane>>, StartupError> {
    let Some(approval_path) = options.sync_transport.as_deref() else {
        return Ok(None);
    };
    let Some(scope) = scope else {
        // 私有配置没批准范围就是「关闭」：给了审批文档也不装配，但要让操作者看见原因。
        return Err(StartupError::SyncPlane);
    };
    let (Some(workbench), Some(audit), Some(canonical_identity)) =
        (workbench, audit, canonical_identity)
    else {
        return Err(StartupError::SyncPlane);
    };
    let scope: &'static SyncScope = Box::leak(Box::new(scope));
    let approval =
        SyncTransportApproval::load(approval_path).map_err(|_| StartupError::SyncPlane)?;
    let mut isolated: Vec<&Path> = vec![options.vault.as_path(), options.assets.as_path()];
    isolated.extend(options.writer_data.as_deref());
    isolated.extend(reserved.iter().map(std::path::PathBuf::as_path));
    let approved = ApprovedTransport::approve(scope, approval, &isolated)
        .map_err(|_| StartupError::SyncPlane)?;
    let approved: &'static ApprovedTransport<'static> = Box::leak(Box::new(approved));
    let runtime = PinnedRuntime::from_approval(approved).map_err(|_| StartupError::SyncPlane)?;
    // 受控后端只能由 launcher 探测得到；探测不到就不启动任何第三方程序，也不监听。
    let backend = verify_backend(approved).map_err(|_| StartupError::SyncPlane)?;
    let transport = SyncTransportOwner::open_existing(approved, runtime, backend)
        .map_err(|_| StartupError::SyncPlane)?;
    let roots = approved.roots();
    let bridge = BridgeState::open_existing(
        &roots.plane_root().join(BRIDGE_DIR),
        binding(approved, canonical_identity),
    )
    .map_err(|_| StartupError::SyncPlane)?;
    let plane = SyncPlane::open(PlaneInputs {
        approval: approved,
        transport,
        bridge,
        workbench: Arc::clone(workbench),
        audit: Arc::clone(audit),
        plane_root: roots.plane_root().to_path_buf(),
    })
    .map_err(|_| StartupError::SyncPlane)?;
    Ok(Some(plane))
}

/// Bridge 持久状态的绑定：链身份、Vault 身份与两个根 token 全部由审批与已核验根身份派生；
/// `canonical_identity` 是可信启动层核验过的 canonical 根 `(dev, ino)`（`ServingRoots`）。
pub(super) fn binding(approved: &ApprovedTransport<'_>, canonical_identity: (u64, u64)) -> Binding {
    let scope = approved.binding();
    let roots = approved.roots();
    Binding {
        link_id: scope.link_id,
        canonical_vault: vault_identity(&scope.vault_uuid.to_string()),
        remote_vault: scope.remote_vault,
        canonical_root_token: root_token("canonical", canonical_identity),
        mirror_root_token: root_token("mirror", roots.mirror_identity()),
        scope_hash: scope.scope_hash,
        scope_epoch: scope.scope_epoch,
    }
}

/// 根 token：域分隔 sha256(角色 ‖ dev ‖ ino)。
pub(super) fn root_token(role: &str, identity: (u64, u64)) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"studiolo.sync-plane.root.v1\0");
    hash.update(role.as_bytes());
    hash.update(identity.0.to_be_bytes());
    hash.update(identity.1.to_be_bytes());
    hash.finalize().into()
}

/// Vault 身份的 16 字节投影：域分隔 sha256 的前 16 字节。
pub(super) fn vault_identity(uuid: &str) -> [u8; 16] {
    let mut hash = Sha256::new();
    hash.update(b"studiolo.sync-plane.vault.v1\0");
    hash.update(uuid.as_bytes());
    let digest: [u8; 32] = hash.finalize().into();
    let mut identity = [0u8; 16];
    identity.copy_from_slice(&digest[..16]);
    identity
}
