//! 备份的可信生产者、它的专用 socket，和健康页用的备份事实。
//!
//! 本 Module **不新增任何对外 HTTP 入口**：备份的结论由既有健康页（`/api/v1/health`，走既有鉴权门禁）承载，
//! 这里只给事实（`BackupJobs`、`RepositoryChecks`），界面上的句子由 `crate::health` 出。
//!
//! 生产者只在启动给了私有备份配置（`--backup-config`，`backup` 和 `backup_producer` 两段成对）时才启用：它在固定路径上
//! 监听一个只对独立 worker 账号开放的 Unix socket，答「下一个待办」「该不该做一次封存」和
//! 「该不该抽查仓库」，收抽查结果，并把带键的审计请求交给原准入端口；只使用启动保留的原账本。
//!
//! 健康页的「恢复点年龄」「备份作业」「备份仓库检查」三节经 `BackupJobsProbe` 从生产者取事实：只读原账本。

pub mod admission;
pub mod audit;
mod check_evidence;
mod checks;
mod evidence;
#[cfg(target_os = "linux")]
mod listener;
pub mod producer;
mod queue;
#[cfg(target_os = "linux")]
mod service;

pub use check_evidence::{
    CheckMark, CheckProblem, CheckVerdict, RepositoryCheckEvidence, RepositoryChecks, RunningCheck,
};
pub use evidence::{
    BackupJobs, BackupJobsEvidence, BackupJobsProbe, FailureMark, JobFailure, JobMark, JobsVerdict,
    OpenJob, RecoveryMark,
};
#[cfg(target_os = "linux")]
pub use listener::ListenError;
pub use producer::{BackupProducer, ProducerPlan};
#[cfg(target_os = "linux")]
pub use service::BackupService;
