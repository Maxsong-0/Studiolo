//! 传输运行审批：默认关闭；把 runtime/service/根/UID/预算封闭成一个不可公开构造的能力。
//!
//! 本 Module 不启动进程、不打开运行时文件、不读环境，也不声称受控 Linux 隔离已经具备。

mod capability;
mod document;
mod types;

pub use capability::ApprovedTransport;
/// Studiolo 自己生成冲突／回滚副本时用的固定设备名（local-server `sync_plane::naming` 取这里的母本）；
/// 审批的 `device_name` 不许与它大小写不敏感地相同，否则两侧副本同名或撞名。
pub const CANONICAL_CONFLICT_DEVICE: &str = "studiolo-canonical";
pub use document::{SyncTransportApproval, excluded_folder_acceptable};
pub use types::{
    ManualQuota, TransportBackend, TransportBudget, TransportRoots, TransportRuntime,
    TransportSyncMode,
};
