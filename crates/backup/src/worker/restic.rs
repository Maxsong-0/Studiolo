//! 固定 restic 包与封闭参数。这里只组装参数向量，不启动任何进程。
//!
//! 没有任何入口可以传入任意 shell 命令：可变部分只有 tag、快照 ID、
//! 绝对路径与仓库定位符，每一项都要通过封闭字符集检查。
//! 追加凭据只属于 BackupWorker；恢复与保留清理各用独立身份。

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

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

/// 备份仓库定位符；只接受绝对路径或固定前缀的远端形式。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RepositoryRef(String);

impl RepositoryRef {
    /// 解析仓库定位符。
    pub fn parse(value: impl Into<String>) -> Result<Self, ResticError> {
        let value = value.into();
        if !is_closed_token(&value) || value.len() > 512 {
            return Err(ResticError::ForbiddenArgument);
        }
        let accepted = value.starts_with('/')
            || value.starts_with("rest:https://")
            || value.starts_with("sftp:")
            || value.starts_with("s3:https://");
        if !accepted {
            return Err(ResticError::ForbiddenArgument);
        }
        Ok(Self(value))
    }

    /// 返回定位符文本。
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// 作业 tag；只允许小写字母、数字、点、下划线与连字符。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct JobTag(String);

impl JobTag {
    /// 解析 tag。
    pub fn parse(value: impl Into<String>) -> Result<Self, ResticError> {
        let value = value.into();
        let shaped = !value.is_empty()
            && value.len() <= 64
            && value.bytes().all(|byte| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || matches!(byte, b'.' | b'_' | b'-')
            });
        if shaped {
            Ok(Self(value))
        } else {
            Err(ResticError::ForbiddenArgument)
        }
    }

    /// 返回 tag 文本。
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// 远端快照标识；8 至 64 位小写十六进制。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SnapshotRef(String);

impl SnapshotRef {
    /// 解析快照标识。
    pub fn parse(value: impl Into<String>) -> Result<Self, ResticError> {
        let value = value.into();
        let shaped = (8..=64).contains(&value.len())
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
        if shaped {
            Ok(Self(value))
        } else {
            Err(ResticError::ForbiddenArgument)
        }
    }

    /// 返回快照标识文本。
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
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

fn absolute_argument(path: &Path) -> Result<String, ResticError> {
    let text = path.to_str().ok_or(ResticError::ForbiddenArgument)?;
    if !path.is_absolute() || text.contains("/../") || text.ends_with("/..") {
        return Err(ResticError::ForbiddenArgument);
    }
    Ok(text.to_owned())
}

/// 封闭字符集：控制字符、空白与 shell 元字符一律拒绝。
fn is_closed_token(value: &str) -> bool {
    !value.is_empty()
        && !value.chars().any(|character| {
            character.is_control()
                || character.is_whitespace()
                || matches!(
                    character,
                    ';' | '&'
                        | '|'
                        | '$'
                        | '`'
                        | '<'
                        | '>'
                        | '('
                        | ')'
                        | '{'
                        | '}'
                        | '['
                        | ']'
                        | '*'
                        | '?'
                        | '!'
                        | '\\'
                        | '\''
                        | '"'
                        | '\u{0}'
                )
        })
}

/// 一次真实调用的结果。拿不到真实仓库时返回 `Unverified`，不伪装成功。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResticOutcome {
    /// 进程已退出；退出码只是中间证据。
    Exited {
        /// 是否以 0 退出。
        status_zero: bool,
        /// 自报的对象数。
        reported_objects: u64,
    },
    /// 明确未验证；不是成功也不是失败。
    Unverified {
        /// 未验证的原因。
        reason: UnverifiedReason,
    },
}

/// 未验证的原因。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnverifiedReason {
    /// 尚无真实备份仓库。
    NoRealRepository,
    /// 备份目标不可达。
    TargetUnreachable,
    /// 目标的不可变能力无法证明。
    ImmutabilityUnprovable,
}

/// 执行 restic 调用的 Interface；本 crate 不实现真实执行。
pub trait ResticRunner {
    /// 执行一次已检查的调用。
    fn run(&self, invocation: &ResticInvocation) -> Result<ResticOutcome, ResticError>;
}

/// 没有接入真实仓库时使用的 runner：一律返回明确未验证。
#[derive(Clone, Copy, Debug, Default)]
pub struct UnconfiguredResticRunner;

impl ResticRunner for UnconfiguredResticRunner {
    fn run(&self, _invocation: &ResticInvocation) -> Result<ResticOutcome, ResticError> {
        Ok(ResticOutcome::Unverified {
            reason: UnverifiedReason::NoRealRepository,
        })
    }
}

/// restic 参数与身份的失败模式。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResticError {
    /// 参数包含 shell 元字符、空白或控制字符，或不是绝对路径。
    ForbiddenArgument,
    /// 追加凭据被 BackupWorker 之外的组件持有。
    CredentialHolderForbidden {
        /// 越界的持有者。
        holder: CredentialHolder,
    },
    /// 身份与命令要求不符。
    IdentityMismatch {
        /// 命令要求的身份。
        expected: ResticIdentity,
        /// 实际传入的身份。
        found: ResticIdentity,
    },
    /// 调用真实执行失败。
    ExecutionFailed,
}

impl std::fmt::Display for ResticError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ForbiddenArgument => formatter.write_str("restic 参数不在封闭集合内"),
            Self::CredentialHolderForbidden { holder } => {
                write!(formatter, "{holder:?} 不得持有备份仓库追加凭据")
            }
            Self::IdentityMismatch { expected, found } => {
                write!(formatter, "命令要求身份 {expected:?}，实际为 {found:?}")
            }
            Self::ExecutionFailed => formatter.write_str("restic 调用执行失败"),
        }
    }
}

impl std::error::Error for ResticError {}
