//! launcher 应答到 owner 侧类型的纯映射；不做 IO，不猜：形状不符一律 `ProtocolViolation`。

use super::super::cgroup::CgroupLimits;
use super::super::launcher::{IsolationReport, LaunchAcknowledgement, ReleaseToken};
use super::super::protocol::{LaunchReport, PROTOCOL_VERSION, ProbeReport};
use super::super::{ContainmentGap, IdentityMatch};
use crate::owned_run::{BackendDescriptor, ContainerIdentity, ExitObservation, RunId};

/// 探测应答逐项核对：协议版本、nonce 回显、三项前置、payload 身份与批准值相等。
///
/// # 错误
/// 版本或 nonce 不符返回 `ProtocolViolation`；三项前置各自返回 `CgroupV2Missing`／
/// `DiskQuotaMissing`／`DedicatedIdentityMissing`；身份不等返回 `IdentityMismatch`。
pub(super) fn probe_acceptable(
    report: &ProbeReport,
    nonce: [u8; 32],
    backend: &BackendDescriptor,
) -> Result<(), ContainmentGap> {
    if report.protocol_version != PROTOCOL_VERSION || report.nonce != nonce {
        return Err(ContainmentGap::ProtocolViolation);
    }
    if !report.cgroup_v2 {
        return Err(ContainmentGap::CgroupV2Missing);
    }
    if !report.loop_quota {
        return Err(ContainmentGap::DiskQuotaMissing);
    }
    if !report.dedicated_identity {
        return Err(ContainmentGap::DedicatedIdentityMissing);
    }
    if report.payload_uid != backend.payload_uid || report.payload_gid != backend.payload_gid {
        return Err(ContainmentGap::IdentityMismatch);
    }
    Ok(())
}

/// `launch` 应答投影成屏障事实；放行凭证绑定运行标识与 launcher 报回的一次性会话。
pub(super) fn acknowledgement(
    run_id: RunId,
    container: ContainerIdentity,
    report: &LaunchReport,
) -> LaunchAcknowledgement {
    let limits = CgroupLimits {
        memory_max: report.limits.memory_max,
        memory_swap_max: report.limits.memory_swap_max,
        pids_max: report.limits.pids_max,
        cpu_max: report.limits.cpu_max,
    };
    let isolation = IsolationReport {
        mount_namespace: report.isolation.mount_namespace,
        pid_namespace: report.isolation.pid_namespace,
        dedicated_identity: report.isolation.dedicated_identity,
        dropped_capabilities: report.isolation.dropped_capabilities,
        payload_cannot_write_cgroup: report.isolation.payload_cannot_write_cgroup,
        final_view_identity_checked: report.isolation.final_view_identity_checked,
        disk_quota_enforced: report.isolation.disk_quota_enforced,
    };
    let release = ReleaseToken::new(run_id, container.launcher_session);
    LaunchAcknowledgement {
        container,
        limits,
        isolation,
        diagnostics: report.diagnostics,
        release,
    }
}

/// `reap` 应答：没取得等待结果就是 `NotObserved`；退出码与信号二选一，两者都给或都缺都不合法。
pub(super) fn exit_observation(
    observed: bool,
    code: Option<i32>,
    signal: Option<i32>,
) -> Result<ExitObservation, ContainmentGap> {
    match (observed, code, signal) {
        (false, _, _) => Ok(ExitObservation::NotObserved),
        (true, Some(code), None) => Ok(ExitObservation::Code(code)),
        (true, None, Some(signal)) => Ok(ExitObservation::Signal(signal)),
        _ => Err(ContainmentGap::ProtocolViolation),
    }
}

/// `match` 应答的五个固定字面量；其它一律不认。
pub(super) fn identity_match(identity: &str) -> Result<IdentityMatch, ContainmentGap> {
    Ok(match identity {
        "exact" => IdentityMatch::Exact,
        "unit_missing" => IdentityMatch::UnitMissing,
        "unit_replaced" => IdentityMatch::UnitReplaced,
        "cgroup_reused" => IdentityMatch::CgroupReused,
        "boot_changed" => IdentityMatch::BootChanged,
        _ => return Err(ContainmentGap::ProtocolViolation),
    })
}
