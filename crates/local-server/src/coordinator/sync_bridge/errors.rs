//! Bridge 接缝的固定失败分类，不带正文或路径，不创建写者。

use super::{BridgeSeamError, BrokerError, FsVaultBroker};
use studiolo_vault_broker::FileOperationPhase;

/// 尚未碰 Broker 写入就失败（锁中毒、根漂移、身份不符、请求元数据不合法）的统一分类，不带原因正文。
pub(super) fn not_attempted<E>(_: E) -> BridgeSeamError {
    BridgeSeamError::NotAttempted
}

/// 文件操作仍停在准备阶段（尚无可见变更）。
pub(super) fn before_visible(broker: &FsVaultBroker) -> bool {
    broker.file_operation_status().is_ok_and(|status| {
        status.is_some_and(|status| {
            matches!(
                status.phase,
                FileOperationPhase::Preparing | FileOperationPhase::Prepared
            )
        })
    })
}

/// 观察阶段的失败分类：安全诊断与范围问题是 `Incomplete`，其余都是写者不可用。
pub(super) fn observation_error(error: BrokerError) -> BridgeSeamError {
    match error {
        BrokerError::IncompleteCapture
        | BrokerError::InvalidArtifact(_)
        | BrokerError::ProtectedPath(_)
        | BrokerError::UnsafeNode { .. }
        | BrokerError::PathCollision { .. } => BridgeSeamError::Incomplete,
        _ => BridgeSeamError::Unavailable,
    }
}

/// 安装之前（重放查询、准备）的失败分类：CAS 拒绝已持久为终态；受保护路径、写者预算超限与原键已明确取消都肯定没写入；未决与键冲突都当未知。
pub(super) fn write_error(error: BrokerError) -> BridgeSeamError {
    match error {
        BrokerError::RevisionConflict { .. } => BridgeSeamError::Rejected,
        BrokerError::ProtectedPath(_)
        | BrokerError::WriterLimitExceeded
        | BrokerError::MutationCancelled => BridgeSeamError::NotAttempted,
        BrokerError::MutationUnresolved | BrokerError::IdempotencyConflict => {
            BridgeSeamError::Unknown
        }
        _ => BridgeSeamError::Unavailable,
    }
}

/// 安装之后的失败分类：CAS 拒绝是权威终态；其余只要写者仍有未决就是未知。
pub(super) fn install_error(broker: &FsVaultBroker, error: BrokerError) -> BridgeSeamError {
    let status = broker.write_status();
    match (error, status.map_or(true, |status| status.unresolved)) {
        (BrokerError::RevisionConflict { .. }, _) => BridgeSeamError::Rejected,
        (_, true) => BridgeSeamError::Unknown,
        (_, false) => BridgeSeamError::Unavailable,
    }
}
