//! restic 参数与身份的失败模式。

use super::{CredentialHolder, ResticIdentity};

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
