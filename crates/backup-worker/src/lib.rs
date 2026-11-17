//! 独立 worker 的真实进程边界；App 不依赖本包，也不获得凭据或命令执行能力。
#![cfg(target_os = "linux")]

mod audit;
mod config;
mod credentials;
mod job;
mod package;
mod process;
mod repository_check;
mod runner;
mod runtime;
pub mod serve;
mod source;
mod temporary;

pub use audit::{WorkerAuditClient, WorkerQueueClient};
pub use config::WorkerConfig;
pub use credentials::ResticCredentials;
pub use job::ApprovedJob;
pub use package::ResticPackage;
pub use runner::{AppendRunner, FinalManifestProbe};

/// 固定错误类别，不包含子进程正文、环境值、路径或凭据。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcessError {
    /// 固定可执行文件的所有权、权限、身份或摘要不符。
    Package,
    /// 调用不满足固定角色、仓库或参数约束。
    Configuration,
    /// 原封存引用、内容或源路径身份不符。
    Source,
    /// 无法启动已批准的固定包。
    Spawn,
    /// 输出越界，已终止并回收本次进程组。
    OutputLimit,
    /// 调用到期，已终止并回收本次进程组。
    Deadline,
    /// 输出管道或进程状态读取失败。
    Io,
    /// 无法证实子进程已回收；调用方必须终止 worker，不能开始下一个作业。
    CleanupFailed,
    /// 子进程输出与固定版本的协议不符。
    Protocol,
    /// 调用方要求让路（仓库抽查遇到待办的备份作业），已终止并回收本次进程组。
    Interrupted,
}
impl std::fmt::Display for ProcessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "备份子进程未完成：{self:?}")
    }
}
impl std::error::Error for ProcessError {}
type Result<T> = std::result::Result<T, ProcessError>;

#[cfg(test)]
#[path = "../tests/support/fixtures.rs"]
mod fixtures;
#[cfg(test)]
#[path = "../tests/support/source_binding/fixture.rs"]
mod seal_fixture;
