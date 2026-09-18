//! 恢复到**新目录**的物理重绑定，由 Broker 自己实现。
//!
//! 主 binding 与 L0 绑定含设备/inode，把封存内容复制到新目录之后，
//! 普通 `open_existing` 必然拒绝。本模块先完整验证封存内容与所有逻辑引用，
//! 再保留已验证的 Vault UUID 与逻辑 `pair_id`，重新建立新目录的物理绑定，
//! 并记录恢复 lineage。
//!
//! 明确不提供、也不依赖的路径：CLI 手改 JSON、普通 `initialize` 自动修复、
//! 复用单事务维护票据。中途失败只会留下隔离且不能服务的新目录。

mod lineage;
mod mirror;
mod paths;
mod rebind;
mod verify;

pub use lineage::{LineageState, PhysicalId, RestoreLineage};

use crate::{BrokerError, VaultUuid};
use std::path::Path;

/// 已启用的 `maintenance-v1` 是否被重绑定。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaintenanceCoverage {
    /// 已启用并已重绑定。
    Rebound,
    /// 该 Vault 未启用离线维护。
    NotEnabled,
}

/// 本次重绑定实际覆盖到的位置。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RebindCoverage {
    /// canonical 侧 `.workbench/broker-binding.json`。
    pub canonical_binding: bool,
    /// data 侧 `broker-binding.json`。
    pub data_binding: bool,
    /// `history/meta-v1` 目录组。
    pub history_meta_v1: bool,
    /// 已启用的 `maintenance-v1`。
    pub maintenance_v1: MaintenanceCoverage,
}

impl RebindCoverage {
    /// 是否覆盖了两处主 binding 与 `history/meta-v1`，而不是只改主根设备号。
    #[must_use]
    pub fn covers_required_bindings(&self) -> bool {
        self.canonical_binding && self.data_binding && self.history_meta_v1
    }
}

/// 一次新目录重绑定请求。
#[derive(Clone, Debug)]
pub struct NewRootRebind<'a> {
    /// 新的 canonical 根（已由恢复流程写入封存内容）。
    pub new_vault_root: &'a Path,
    /// 新的 data 根。
    pub new_data_root: &'a Path,
    /// 已验证的 Vault 身份，必须与封存内容一致。
    pub expected_vault_uuid: VaultUuid,
    /// 已验证的逻辑配对标识，原样保留。
    pub expected_pair_id: VaultUuid,
    /// 本次恢复所基于的 checkpoint 标识。
    pub checkpoint_id: String,
    /// 上一次恢复的 lineage 标识。
    pub parent_lineage: Option<String>,
    /// 是否明确接手上一次中途失败留下的新目录。
    pub resume_interrupted: bool,
}

/// 重绑定成功后的回执。
#[derive(Clone, Debug)]
pub struct RestoreReceipt {
    /// 保留下来的 Vault 身份。
    pub vault_uuid: VaultUuid,
    /// 保留下来的逻辑配对标识。
    pub pair_id: VaultUuid,
    /// 覆盖情况。
    pub coverage: RebindCoverage,
    /// 已完成的 lineage 记录。
    pub lineage: RestoreLineage,
}

/// 只在测试里使用的中断点；生产路径一律 `None`。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RestoreFault {
    /// 不中断。
    None,
    /// lineage 落盘之后中断。
    AfterLineage,
    /// `history/meta-v1` 重绑定之后中断。
    AfterHistory,
    /// `maintenance-v1` 重绑定之后中断。
    AfterMaintenance,
    /// data 侧主 binding 改写之后中断。
    AfterDataBinding,
    /// canonical 侧主 binding 改写之后中断。
    AfterCanonicalBinding,
}

impl RestoreFault {
    fn trip(self, point: Self) -> Result<(), RestoreError> {
        if self == point {
            return Err(RestoreError::InterruptedByFault);
        }
        Ok(())
    }
}

/// 新目录恢复的失败模式。
#[derive(Debug)]
pub enum RestoreError {
    /// 底层 Broker 错误。
    Broker(BrokerError),
    /// 新目录缺少封存内容或存在非目录节点。
    NewRootIncomplete,
    /// 新 canonical 根已被其他写者占用。
    NewRootBusy,
    /// marker 或 binding 中的 Vault 身份与期望不符。
    VaultIdentityMismatch,
    /// 逻辑 `pair_id` 与期望不符。
    PairIdMismatch,
    /// 两处主 binding 内容不一致。
    BindingsDiverged,
    /// 某处逻辑引用没有指向同一个 `pair_id`。
    LogicalReferenceMismatch {
        /// 不一致的引用位置。
        reference: &'static str,
    },
    /// `maintenance-v1` 处于半初始化状态。
    MaintenanceHalfInitialized,
    /// 上一次恢复中途失败，必须显式接手。
    Interrupted,
    /// lineage 记录版本未知。
    UnsupportedLineage,
    /// checkpoint 标识不合法。
    InvalidCheckpointId,
    /// 测试中断点触发。
    InterruptedByFault,
}

impl std::fmt::Display for RestoreError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Broker(error) => write!(formatter, "恢复底层失败：{error}"),
            Self::NewRootIncomplete => formatter.write_str("新目录缺少封存内容"),
            Self::NewRootBusy => formatter.write_str("新 canonical 根已被占用"),
            Self::VaultIdentityMismatch => formatter.write_str("Vault 身份与期望不符"),
            Self::PairIdMismatch => formatter.write_str("逻辑 pair_id 与期望不符"),
            Self::BindingsDiverged => formatter.write_str("两处主 binding 不一致"),
            Self::LogicalReferenceMismatch { reference } => {
                write!(formatter, "逻辑引用 {reference} 不指向同一 pair_id")
            }
            Self::MaintenanceHalfInitialized => {
                formatter.write_str("maintenance-v1 处于半初始化状态")
            }
            Self::Interrupted => formatter.write_str("上一次恢复中途失败，需要显式接手"),
            Self::UnsupportedLineage => formatter.write_str("恢复 lineage 版本未知"),
            Self::InvalidCheckpointId => formatter.write_str("checkpoint 标识不合法"),
            Self::InterruptedByFault => formatter.write_str("恢复在注入的中断点停止"),
        }
    }
}

impl std::error::Error for RestoreError {}

/// 读取新 data 根上的恢复 lineage；没有恢复过返回 `None`。
pub fn read_restore_lineage(new_data_root: &Path) -> Result<Option<RestoreLineage>, RestoreError> {
    let data_path = paths::absolute(new_data_root)?;
    let data = paths::walk(&data_path)?;
    rebind::read_lineage(&data)
}

/// 把已复制到新目录的封存内容重新绑定成一个可服务的配对存储。
pub fn rebind_new_roots(request: &NewRootRebind<'_>) -> Result<RestoreReceipt, RestoreError> {
    run(request, RestoreFault::None)
}

pub(crate) fn run(
    request: &NewRootRebind<'_>,
    fault: RestoreFault,
) -> Result<RestoreReceipt, RestoreError> {
    if request.checkpoint_id.trim().is_empty()
        || request.checkpoint_id.len() > 128
        || request.checkpoint_id.chars().any(char::is_control)
    {
        return Err(RestoreError::InvalidCheckpointId);
    }
    let vault_path = paths::absolute(request.new_vault_root)?;
    let data_path = paths::absolute(request.new_data_root)?;
    paths::disjoint(&vault_path, &data_path)?;
    let root = paths::walk(&vault_path)?;
    // 第二写者仍在运行时立刻失败，不会边改绑定边被写。
    let _lease = paths::acquire(&root)?;
    let data = paths::walk(&data_path)?;

    let interrupted = rebind::read_lineage(&data)?
        .is_some_and(|existing| existing.state == LineageState::Rebinding);
    if interrupted && !request.resume_interrupted {
        return Err(RestoreError::Interrupted);
    }

    let verified = verify::verify(
        &root,
        &data,
        request.expected_vault_uuid,
        request.expected_pair_id,
        interrupted && request.resume_interrupted,
    )?;
    let rebound = rebind::rebind(
        &root,
        &data,
        &verified,
        &request.checkpoint_id,
        request.parent_lineage.clone(),
        fault,
    )?;
    Ok(RestoreReceipt {
        vault_uuid: request.expected_vault_uuid,
        pair_id: request.expected_pair_id,
        coverage: rebound.coverage,
        lineage: rebound.lineage,
    })
}

#[cfg(test)]
mod tests;
