//! 双向 Bridge 平面的组合根（ADR-025 D1）：只在显式 `--sync-transport` 且私有配置带
//! `sync_scope` 时装配；缺任一项即 `Ok(None)`，合入 main 不改变生产行为。
//!
//! 装配顺序：审批文档 → 借用范围追加审批（根 pins）→ 固定运行时 → 经 launcher 探测受控后端
//! → 打开传输运行槽 → 打开 Bridge 持久状态 → 组装平面。任一步失败返回
//! `StartupError::SyncPlane`，不监听。审批与范围经 `Box::leak` 取得进程生命期引用：
//! 平面与进程同生死，这两份只读能力也是。

use super::StartupError;
use crate::options::Options;
use std::io::Write as _;
use std::{path::Path, sync::Arc};
use studiolo_audit_ledger::AuditLedger;
use studiolo_host_config::SyncScope;
use studiolo_host_config::sync_transport::{ApprovedTransport, SyncTransportApproval};
use studiolo_local_server::CanonicalWorkbench;
use studiolo_local_server::sync_plane::{
    BRIDGE_DIR, PlaneInputs, SyncPlane, bridge_binding, bridge_limits,
};
use studiolo_sync_bridge::sqlite::BridgeState;
use studiolo_sync_transport::linux::verify_backend;
use studiolo_sync_transport::owned_run::{PinnedRuntime, SyncTransportOwner};

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
    // 传输私有根、视图根、平面根三者之间，以及它们与全部权威根（canonical、assets、
    // writer-data、策略目录、黑板控制目录、Auth／Audit／配置目录）的两两不重叠都在
    // `approve` 里按真实祖先身份核对；这里只负责把权威根申报齐全，漏报即等于没核。
    let mut isolated: Vec<&Path> = vec![options.vault.as_path(), options.assets.as_path()];
    isolated.extend(options.writer_data.as_deref());
    isolated.extend(options.policy_data.as_deref());
    isolated.extend(options.agent_data.as_deref());
    isolated.extend(options.board.as_ref().map(|board| board.data.as_path()));
    isolated.extend(reserved.iter().map(std::path::PathBuf::as_path));
    let approved = ApprovedTransport::approve(scope, approval, &isolated)
        .map_err(|_| StartupError::SyncPlane)?;
    let approved: &'static ApprovedTransport<'static> = Box::leak(Box::new(approved));
    let runtime = PinnedRuntime::from_approval(approved).map_err(|_| StartupError::SyncPlane)?;
    // 受控后端只能由 launcher 探测得到；探测不到就不启动任何第三方程序，也不监听。
    let backend = verify_backend(approved).map_err(|_| StartupError::SyncPlane)?;
    let transport = SyncTransportOwner::open_existing(approved, runtime, backend)
        .map_err(|_| StartupError::SyncPlane)?;
    if transport.discarded_residue() {
        // 上一个进程在换状态的那一下没了（T-239）：丢之前权威记录已核过。只有固定分类，不带路径和内容。
        let _ = writeln!(
            std::io::stderr().lock(),
            "studiolo_bridge component=transport_store outcome=publish_residue_discarded"
        );
    }
    let roots = approved.roots();
    let mut bridge = BridgeState::open_existing(
        &roots.plane_root().join(BRIDGE_DIR),
        bridge_binding(approved, canonical_identity),
    )
    .map_err(|_| StartupError::SyncPlane)?;
    // BUG-047：旧链 `sync-init` 时冻结的路径预算升到本发行的同步路径预算；只升不降，已是目标值不写库。
    // 升了之后旧发行认不出这份持久预算（超过它的上限），退回旧发行前要换回升级前复制的 Bridge 库。
    let limits = bridge_limits(scope);
    if let Some(previous) = bridge
        .raise_capacity(limits)
        .map_err(|_| StartupError::SyncPlane)?
    {
        let _ = writeln!(
            std::io::stderr().lock(),
            "studiolo_bridge component=capacity from={previous} to={}",
            limits.max_paths
        );
    }
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
