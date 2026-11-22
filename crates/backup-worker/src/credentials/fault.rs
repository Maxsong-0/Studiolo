//! 取凭据没成时的固定类别：只说卡在哪一道检查上，不带路径、账号号码和凭据内容（BUG-043）。
//!
//! 以前每一道检查都归成同一个「配置不符」，现场分不清是上级目录、凭据目录的形态、挂载还是文件内容不对，
//! 只能照着源码一级一级去试。类别只进日志；交给作业和抽查的结果照旧是 `ProcessError::Configuration`
//! （超时照旧是 `Deadline`），账本里记的原因一个字不变。
use crate::ProcessError;

/// 取不到凭据的固定类别。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CredentialFault {
    /// 进程账号是 root，或者不是配置里的 worker 账号。
    Account,
    /// 凭据目录的上级（根、`run`、`credentials`）打不开，或者不是 root 专有：属主不对、别人可写、带 ACL。
    Parents,
    /// 凭据目录或三个固定文件里有打不开、读不了的。
    Missing,
    /// 凭据目录不是「只有 root 和 worker 读得到」的两种形态之一（形态见 `acl.rs`）。
    DirectoryAccess,
    /// 凭据不在只读、不会换出到硬盘的内存文件系统上（见 `mount.rs`）。
    Storage,
    /// 凭据文件不是单链接的普通文件，或者不是「只有 root 和 worker 读得到」的两种形态之一。
    FileAccess,
    /// 凭据内容是空的、太长，或者格式不合规。
    Content,
    /// 核对期间凭据目录或文件被换过、改过。
    Changed,
    /// 没在时限内核完。
    Deadline,
}
impl CredentialFault {
    /// 给日志看的一句固定的话。
    #[must_use]
    pub fn explain(self) -> &'static str {
        match self {
            Self::Account => "进程账号是 root，或者不是配置里的 worker 账号",
            Self::Parents => "凭据目录的上级打不开，或者不是 root 专有（属主、权限或 ACL 不对）",
            Self::Missing => "凭据目录或三个固定文件里有打不开、读不了的",
            Self::DirectoryAccess => "凭据目录不是只有 root 和 worker 读得到的两种形态之一",
            Self::Storage => "凭据不在只读、不会换出到硬盘的内存文件系统上",
            Self::FileAccess => "凭据文件不是只有 root 和 worker 读得到的单链接普通文件",
            Self::Content => "凭据内容是空的、太长，或者格式不合规",
            Self::Changed => "核对期间凭据目录或文件变了",
            Self::Deadline => "没在时限内核完",
        }
    }
}
impl std::fmt::Display for CredentialFault {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "取不到备份凭据（{self:?}）：{}", self.explain())
    }
}
impl std::error::Error for CredentialFault {}
impl From<CredentialFault> for ProcessError {
    fn from(fault: CredentialFault) -> Self {
        match fault {
            CredentialFault::Deadline => Self::Deadline,
            _ => Self::Configuration,
        }
    }
}
pub(super) type Result<T> = std::result::Result<T, CredentialFault>;

#[cfg(test)]
#[path = "../../tests/support/credentials_fault.rs"]
mod tests;
