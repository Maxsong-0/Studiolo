//! 固定 restic 包与封闭参数。这里只组装参数向量，不启动任何进程。
//!
//! 没有任何入口可以传入任意 shell 命令：可变部分只有 tag、快照 ID、
//! 绝对路径与仓库定位符，每一项都要通过封闭字符集检查。
//! 追加凭据只属于 BackupWorker；恢复与保留清理各用独立身份。
//!
//! 封闭参数（`arguments` 子模块）、执行 Interface 与结果（`runner` 子模块）、
//! 失败模式（`error` 子模块）按职责拆出，仍经本模块原路径导出。

mod arguments;
mod error;
mod runner;

pub use arguments::{JobTag, RepositoryRef, SnapshotRef};
pub use error::ResticError;
pub use runner::{ResticOutcome, ResticRunner, UnconfiguredResticRunner, UnverifiedReason};

use arguments::{absolute_argument, is_closed_token};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// 固定 restic 包的名字。
pub const RESTIC_PACKAGE_NAME: &str = "restic";
/// 固定 restic 版本；换版本必须改代码并重新验收。
pub const RESTIC_PACKAGE_VERSION: &str = "0.18.2";

/// 备份仓库凭据身份。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResticIdentity {
    /// 只能追加的备份身份，由 BackupWorker 独占。
    Append,
    /// 恢复身份，独立于追加身份。
    Restore,
    /// 保留清理身份，独立于追加身份。
    RetentionCleanup,
}

/// 系统中可能持有凭据的组件。除 BackupWorker 外都不得持有追加凭据。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialHolder {
    /// 独立的 BackupWorker。
    BackupWorker,
    /// 本机应用进程。
    App,
    /// VaultBroker。
    Broker,
    /// Auth。
    Auth,
    /// Audit。
    Audit,
    /// TaskEngine。
    TaskEngine,
}

/// 封闭的命令集合；没有「任意参数」这一支。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResticCommand {
    /// 从只读传输源追加一次备份。
    Backup {
        /// 作业 tag。
        tag: JobTag,
        /// 只读传输源根目录（绝对路径）。
        source_root: PathBuf,
    },
    /// 校验远端对象完整性。
    Check {
        /// 目标快照。
        snapshot: SnapshotRef,
    },
    /// 恢复到新目录。
    Restore {
        /// 目标快照。
        snapshot: SnapshotRef,
        /// 恢复目标根目录（绝对路径，新目录）。
        target_root: PathBuf,
    },
    /// 保留清理。
    Forget {
        /// 保留最近多少个快照。
        keep_last: u32,
    },
}

impl ResticCommand {
    /// 该命令必须使用的身份。
    #[must_use]
    pub fn identity(&self) -> ResticIdentity {
        match self {
            Self::Backup { .. } | Self::Check { .. } => ResticIdentity::Append,
            Self::Restore { .. } => ResticIdentity::Restore,
            Self::Forget { .. } => ResticIdentity::RetentionCleanup,
        }
    }
}

/// 已经过封闭检查的一次调用计划。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResticInvocation {
    package: &'static str,
    version: &'static str,
    repository: RepositoryRef,
    identity: ResticIdentity,
    argv: Vec<String>,
    secret_environment: Vec<&'static str>,
}

/// 秘密只经这些环境变量注入；这里保存的是变量名，不是值。
pub const RESTIC_SECRET_ENVIRONMENT: [&str; 2] = ["RESTIC_PASSWORD_FILE", "RESTIC_REPOSITORY_FILE"];

impl ResticInvocation {
    /// 组装一次调用。持有者不是 BackupWorker、或身份与命令不符即拒绝。
    pub fn plan(
        repository: RepositoryRef,
        command: &ResticCommand,
        holder: CredentialHolder,
        identity: ResticIdentity,
    ) -> Result<Self, ResticError> {
        if holder != CredentialHolder::BackupWorker {
            return Err(ResticError::CredentialHolderForbidden { holder });
        }
        if identity != command.identity() {
            return Err(ResticError::IdentityMismatch {
                expected: command.identity(),
                found: identity,
            });
        }
        let mut argv = vec!["--repo".to_owned(), repository.as_str().to_owned()];
        match command {
            ResticCommand::Backup { tag, source_root } => {
                argv.push("backup".to_owned());
                argv.push("--tag".to_owned());
                argv.push(tag.as_str().to_owned());
                argv.push("--one-file-system".to_owned());
                argv.push(absolute_argument(source_root)?);
            }
            ResticCommand::Check { snapshot } => {
                argv.push("check".to_owned());
                argv.push("--read-data-subset".to_owned());
                argv.push("100%".to_owned());
                argv.push("--snapshot".to_owned());
                argv.push(snapshot.as_str().to_owned());
            }
            ResticCommand::Restore {
                snapshot,
                target_root,
            } => {
                argv.push("restore".to_owned());
                argv.push(snapshot.as_str().to_owned());
                argv.push("--target".to_owned());
                argv.push(absolute_argument(target_root)?);
            }
            ResticCommand::Forget { keep_last } => {
                argv.push("forget".to_owned());
                argv.push("--keep-last".to_owned());
                argv.push(keep_last.to_string());
            }
        }
        for argument in &argv {
            if !is_closed_token(argument) {
                return Err(ResticError::ForbiddenArgument);
            }
        }
        Ok(Self {
            package: RESTIC_PACKAGE_NAME,
            version: RESTIC_PACKAGE_VERSION,
            repository,
            identity,
            argv,
            secret_environment: RESTIC_SECRET_ENVIRONMENT.to_vec(),
        })
    }

    /// 固定包名。
    #[must_use]
    pub fn package(&self) -> &'static str {
        self.package
    }

    /// 固定版本。
    #[must_use]
    pub fn version(&self) -> &'static str {
        self.version
    }

    /// 本次调用使用的身份。
    #[must_use]
    pub fn identity(&self) -> ResticIdentity {
        self.identity
    }

    /// 仓库定位符。
    #[must_use]
    pub fn repository(&self) -> &RepositoryRef {
        &self.repository
    }

    /// 已检查的参数向量。
    #[must_use]
    pub fn argv(&self) -> &[String] {
        &self.argv
    }

    /// 秘密注入所用的环境变量名；这里永远不出现值。
    #[must_use]
    pub fn secret_environment(&self) -> &[&'static str] {
        &self.secret_environment
    }
}
