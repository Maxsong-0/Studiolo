//! OwnedRun：纯 DTO、纯状态迁移、持久 schema v1 与最小封闭 API。
//!
//! 进程成功退出不称同步成功；重开只观察、停止本运行或补原审计，不得自动重新执行。

pub mod approval;
pub mod audit;
pub mod error;
pub mod machine;
pub mod state;
pub mod tickets;
pub mod types;

mod host_approval;
mod lifecycle;
mod owner;
mod quiescence;
mod session;

pub use approval::{
    ApprovedTransport, BackendDescriptor, PINNED_RUNTIME_VERSION, PinnedRuntime, RootPins,
    RunBudget, RuntimePins, TransportBindingFacts, absolute_normal, runtime_digest,
};
pub use audit::{
    AuditPayload, AuditPhaseKind, OutcomeSummary, RequesterSource, RunAuditCandidate,
    TRANSPORT_RUN_ACTION,
};
pub use error::{Result, TransportError};
pub use host_approval::backend_digest;
pub use owner::{SyncTransportOwner, initial_state, initialize_store};
pub(crate) use session::random;
pub use state::{
    AuditReceipt, AuditSlot, OwnerIdentity, QuiescenceEvidence, QuiescenceProof, RunRecord,
    SCHEMA_VERSION, STATE_MAX_BYTES, TransportState,
};
pub use tickets::{OwnedRun, PreparedRun, QuiescentMirror};
pub use types::{
    ActiveSummary, ContainerIdentity, ExitObservation, Quiescence, ResourceCounts, RunDiagnostics,
    RunId, RunObservation, RunOperation, RunPhase, RunStatus, StopReason, SyncMode,
    TerminalSummary,
};

#[cfg(test)]
#[path = "../tests/owned_run_private/mod.rs"]
mod tests;
