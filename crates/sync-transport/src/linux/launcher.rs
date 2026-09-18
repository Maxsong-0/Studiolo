//! 可信启动屏障的纯协议：先建子组再放行，一次放行只能被消费一次。
//!
//! 握手 nonce 只用于本次握手，不是长期认证密钥，也不写入任何公开 DTO 或审计。

use super::ContainmentGap;
use super::cgroup::CgroupLimits;
use crate::owned_run::{ContainerIdentity, RunBudget, RunDiagnostics, RunId, RunOperation};
use std::fmt;

/// 本次握手的一次性 nonce；不可打印、不可序列化、不可比较出内容。
#[derive(Clone, PartialEq, Eq)]
pub struct HandshakeNonce([u8; 32]);

impl HandshakeNonce {
    /// 从已取得的随机字节构造。
    #[must_use]
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// 与对端回传值做定长比较；不暴露内容。
    #[must_use]
    pub fn matches(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl fmt::Debug for HandshakeNonce {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("HandshakeNonce(<redacted>)")
    }
}

/// 一次启动请求；argv 是封闭构造的固定命令，不接受任意字符串。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaunchRequest {
    /// 运行标识。
    pub run_id: RunId,
    /// 运行序号。
    pub sequence: u64,
    /// 固定操作。
    pub operation: RunOperation,
    /// 专用 payload UID。
    pub payload_uid: u32,
    /// 专用 payload GID。
    pub payload_gid: u32,
    /// 运行预算。
    pub budget: RunBudget,
    /// 封闭 argv。
    pub argv: Vec<String>,
    /// 本次握手 nonce。
    pub nonce: HandshakeNonce,
}

/// launcher 实际执行的隔离检查结果；全部为真才允许放行。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IsolationReport {
    /// 已建立独立 mount 视图，且其中不含 canonical、writer-data、Bridge DB、外 stage 与宿主 home。
    pub mount_namespace: bool,
    /// 已建立独立 PID 视图。
    pub pid_namespace: bool,
    /// 已降为专用 payload 身份且无补充组。
    pub dedicated_identity: bool,
    /// 已丢弃 capabilities。
    pub dropped_capabilities: bool,
    /// payload 无 cgroupfs 写权限，也够不到 manager socket / launcher IPC / owner FD。
    pub payload_cannot_write_cgroup: bool,
    /// 最终 mount 视图内已核对镜像/config/runtime 的真实身份与批准映射。
    pub final_view_identity_checked: bool,
    /// 私有状态目录已有实际磁盘 quota 或硬上限。
    pub disk_quota_enforced: bool,
}

/// 任一项不成立就不是隔离；`ProtectSystem`/只读全盘不能替代最小文件系统隔离。
///
/// # Errors
/// 缺少 namespace 隔离返回 `NamespaceIsolationMissing`，缺专用身份返回
/// `DedicatedIdentityMissing`，缺磁盘 quota 返回 `DiskQuotaMissing`。
pub fn isolation_sufficient(report: &IsolationReport) -> Result<(), ContainmentGap> {
    if !report.mount_namespace || !report.pid_namespace || !report.final_view_identity_checked {
        return Err(ContainmentGap::NamespaceIsolationMissing);
    }
    if !report.dedicated_identity
        || !report.dropped_capabilities
        || !report.payload_cannot_write_cgroup
    {
        return Err(ContainmentGap::DedicatedIdentityMissing);
    }
    if !report.disk_quota_enforced {
        return Err(ContainmentGap::DiskQuotaMissing);
    }
    Ok(())
}

/// 一次性放行凭证；按值消费，协议只接受一次。
#[derive(Debug, PartialEq, Eq)]
pub struct ReleaseToken {
    run_id: RunId,
    launcher_session: [u8; 16],
}

impl ReleaseToken {
    /// 只由后端在建立屏障后签发。
    #[must_use]
    pub fn new(run_id: RunId, launcher_session: [u8; 16]) -> Self {
        Self {
            run_id,
            launcher_session,
        }
    }

    /// 放行凭证绑定的运行标识。
    #[must_use]
    pub fn run_id(&self) -> RunId {
        self.run_id
    }

    /// 放行凭证绑定的一次性 launcher 会话。
    #[must_use]
    pub fn launcher_session(&self) -> [u8; 16] {
        self.launcher_session
    }
}

/// 屏障内已建立 payload 子组后返回的完整事实。
#[derive(Debug, PartialEq, Eq)]
pub struct LaunchAcknowledgement {
    /// 实测容器身份。
    pub container: ContainerIdentity,
    /// 实测子组限额。
    pub limits: CgroupLimits,
    /// 实测隔离检查结果。
    pub isolation: IsolationReport,
    /// 仅诊断的 PID/start-time。
    pub diagnostics: RunDiagnostics,
    /// 一次性放行凭证。
    pub release: ReleaseToken,
}
