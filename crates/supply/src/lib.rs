//! Studiolo 供给面内核（M3 首片）。
//!
//! 只服务 [[ADR-013]] 定义的**接入方**：按项目白名单裁剪的只读供给、机器派生的冷启动路牌、
//! ≤2000 token 的硬预算、类型＋实例两级身份、读取审计事件与可重建的读取派生索引。
//! 本 crate 不持有 canonical 写能力，不打开网络端口；HTTP/MCP 外壳在 `studiolo-agent-server`。

pub mod audit;
pub mod budget;
pub mod database;
mod error;
pub mod roots;
pub mod signpost;
mod storage;

pub use budget::{
    BudgetExceeded, HARD_BUDGET_TOKENS, TARGET_BUDGET_TOKENS, check_budget, count_tokens,
};
pub use database::{AgentStore, Instance, IssuedToken, ReadQuery, ReadRecord, validate_type_name};
pub use error::SupplyError;
pub use roots::{Entry, EntryKind, Note, Snapshot, SupplyRoots};
pub use signpost::{Principal, Signpost, SignpostInput};
pub use studiolo_vault_broker::ScanLimits;
