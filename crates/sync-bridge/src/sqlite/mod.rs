//! 固定私有 SQLite 状态 owner；本模块不执行实际同步。
//! 只持有固定私有目录；纯 planner 不引用本模块。禁止导出 SQL/任意路径写入口。

mod audit;
mod audit_events;
mod cache;
mod checkpoint;
pub use checkpoint::{BridgeCheckpointGate, BridgeCheckpointSource, FrozenBridge};
mod cancellation;
mod capacity;
mod capacity_raise;
mod codec;
mod configuration;
mod database;
mod dbload;
mod dbwrite;
mod dispatch;
mod executor_port;
mod input_codec;
mod lookup;
mod model;
mod mutations;
mod opening;
mod outcome;
mod pending;
mod public;
mod receipts;
mod reservation;
mod retirement;
mod settlement;
mod state_codec;
mod storage;
mod validate;

pub use executor_port::StateJournal;
pub use model::{
    ActionStatus, AuditDelivery, BridgeError, CommonObservation, EffectReceipt, ExecutionCheck,
    NoEffectReason, PendingAction, ProgressView, ReceiptBackend, ReceiptOutcome, Reservation,
    StateView,
};
pub use opening::BridgeState;

pub(crate) const INPUT_MAX: usize = 16 * 1024 * 1024;
pub(crate) const STATE_MAX: usize = 64 * 1024 * 1024;
pub(crate) const RECEIPT_MAX: usize = 8192;
/// 一批的路径数与动作数上限，就是同步路径预算（BUG-047）。
pub(crate) const MAX_PATHS: usize = studiolo_core::SYNC_PATH_BUDGET;

#[cfg(test)]
#[path = "../../tests/sqlite_private/mod.rs"]
mod tests;
