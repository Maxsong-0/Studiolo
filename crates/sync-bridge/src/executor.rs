//! 双向 Bridge 执行器与 MirrorCAS：只顺序化门禁，不持有文件系统、数据库或网络权限。
//!
//! 本模块是纯逻辑核：真实落盘、真实审计 append 与退休由端口背后的持久 owner 负责。
//! 静止能力来自 [`MirrorQuiescence`]（后续接 SyncTransportOwner 的 `QuiescentMirror`）；
//! 执行器绝不自造静止证明，也不在缺少静止证据时产生任何删除。

mod cas;
mod deletion;
mod drive;
mod error;
mod journal;
mod quiescence;
mod transfer;

pub use cas::{CasFault, CasLimits, MirrorCas, ObjectId, StageUsage};
pub use deletion::{DeletionAudit, DeletionClearance, audit_batch};
pub use drive::{ExecutionPorts, ExecutionReport, execute};
pub use error::ExecutorError;
pub use journal::{
    AuditProgress, CommonRecheck, ExecutionRecheck, ExecutionWitness, IssuedJob, JobOutcome,
    JobRecord, JobState, JournalFault, JournalSnapshot, NoEffectCause, PlanJournal, WitnessFault,
};
pub use quiescence::{
    MirrorQuiescence, MirrorQuiescenceProof, Quiesced, QuiescenceEvidence, QuiescenceFault,
    QuiescenceRequest,
};
pub use transfer::{JobBackend, MirrorContent, MirrorObjectSink, TransferFault};

#[cfg(test)]
#[path = "../tests/executor_private/mod.rs"]
mod tests;
