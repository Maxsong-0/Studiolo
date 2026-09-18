//! 唯一把传输 owner 接到已批准配置与权威 AuditLedger 的地方。
//!
//! 本 Module 自己不取得任何进程、cgroup 或文件能力：批准来自 host-config 的封闭能力，
//! 运行能力来自 `studiolo-sync-transport`，审计走既有权威账本。配置查询也不在这里拿 IO。

mod audit;
mod projection;

pub use audit::{
    TransportAuditError, flush_budget, flush_transport_audit, transport_event, verify_receipt,
};
pub use projection::{TransportRunView, TransportStatusView, status_view};

use std::path::Path;
use studiolo_host_config::{
    ConfigError, SyncScope,
    sync_transport::{ApprovedTransport, SyncTransportApproval},
};

/// 由已批准范围加主人显式审批文档组装传输能力。
///
/// `reserved_roots` 必须由装配方申报：canonical、writer-data、Auth/Audit、Bridge DB、
/// Headless config 与外 stage 都要传进来，传空切片只代表没有申报，不代表已证明不重叠。
///
/// # Errors
/// 审批文档不合法、默认关闭或根不满足私有约束时返回 `ConfigError`。
pub fn approve_transport<'scope>(
    scope: &'scope SyncScope,
    approval: &[u8],
    reserved_roots: &[&Path],
) -> Result<ApprovedTransport<'scope>, ConfigError> {
    ApprovedTransport::approve(
        scope,
        SyncTransportApproval::parse(approval)?,
        reserved_roots,
    )
}

#[cfg(test)]
#[path = "../tests/sync_transport_private/owner.rs"]
mod tests;
