//! 传输运行审批：默认关闭；把 runtime/service/根/UID/预算封闭成一个不可公开构造的能力。
//!
//! 本 Module 不启动进程、不打开运行时文件、不读环境，也不声称受控 Linux 隔离已经具备。

mod capability;
mod document;
mod types;

pub use capability::ApprovedTransport;
pub use document::SyncTransportApproval;
pub use types::{
    ManualQuota, TransportBackend, TransportBudget, TransportRoots, TransportRuntime,
    TransportSyncMode,
};
