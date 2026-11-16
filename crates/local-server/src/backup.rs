//! 备份只读投影、可信生产者和它的专用 socket。
//!
//! 本 Module **不新增任何对外 HTTP 入口**：显示投影只把 `studiolo-backup` 的判定结果翻译成
//! 界面要显示的句子，由既有健康页（`/api/v1/health`，走既有鉴权门禁）承载，不读磁盘、不连远端、
//! 不碰配置。
//!
//! 生产者只在私有配置里有 `backup` 和 `backup_producer` 两段时才启用：它在固定路径上
//! 监听一个只对独立 worker 账号开放的 Unix socket，答「下一个待办」和「该不该做一次封存」，
//! 并把带键的审计请求交给原准入端口；审计端口只使用启动保留的原账本。
//!
//! 健康页的「备份作业」一节经 `BackupJobsProbe` 从生产者取作业事实：只读原账本里的备份事件。

pub mod admission;
pub mod audit;
mod evidence;
#[cfg(target_os = "linux")]
mod listener;
mod model;
pub mod producer;
mod queue;
#[cfg(target_os = "linux")]
mod service;

pub use evidence::{
    BackupEvidence, BackupJobs, BackupJobsEvidence, BackupJobsProbe, BackupSnapshot, FailureMark,
    JobFailure, JobMark, JobsVerdict, OpenJob,
};
#[cfg(target_os = "linux")]
pub use listener::ListenError;
pub use model::{BackupSection, Severity};
pub use producer::{BackupProducer, ProducerPlan};
#[cfg(target_os = "linux")]
pub use service::BackupService;
