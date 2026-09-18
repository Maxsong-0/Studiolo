//! 封存票据、owner 清单与只读传输源的纯类型。
//!
//! 这里没有任何 IO：票据只是「哪些 owner 参与了、导出了什么水位、
//! 传输源在哪、期限到几时」的不可变事实记录。

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use studiolo_core::checkpoint::{
    CheckpointId, CheckpointOwner, InstantMillis, OwnerAssembly, SealedOwnerCheckpoint,
};

/// 逐项标记 present/absent 的 owner 清单；六个 owner 一个都不能缺项。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OwnerRoster(Vec<(CheckpointOwner, OwnerAssembly)>);

impl OwnerRoster {
    /// 按固定顺序构造清单；缺项、乱序或重复一律拒绝。
    pub fn new(
        entries: impl IntoIterator<Item = (CheckpointOwner, OwnerAssembly)>,
    ) -> Result<Self, SealError> {
        let entries: Vec<_> = entries.into_iter().collect();
        if entries.len() != CheckpointOwner::FREEZE_ORDER.len() {
            return Err(SealError::RosterIncomplete);
        }
        for (index, expected) in CheckpointOwner::FREEZE_ORDER.iter().enumerate() {
            if entries[index].0 != *expected {
                return Err(SealError::RosterIncomplete);
            }
        }
        Ok(Self(entries))
    }

    /// 返回某个 owner 的装配情况。
    #[must_use]
    pub fn assembly(&self, owner: CheckpointOwner) -> OwnerAssembly {
        self.0
            .iter()
            .find(|(candidate, _)| *candidate == owner)
            .map_or(OwnerAssembly::Absent, |(_, assembly)| *assembly)
    }

    /// 按固定顺序遍历全部登记项。
    pub fn entries(&self) -> impl Iterator<Item = (CheckpointOwner, OwnerAssembly)> + '_ {
        self.0.iter().copied()
    }
}

/// 传输源的产生方式。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransferSourceKind {
    /// 没有来源排除项，直接使用 owner 导出的只读副本目录。
    DirectExport,
    /// 存在来源排除项，必须先建专用过滤 staging。
    FilteredStaging,
}

/// staging 的物化方式。硬链接活动文件不是不可变视图。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StagingMaterialization {
    /// 固定流式缓冲复制出独立字节。
    StreamedCopy,
    /// 对活动文件建硬链接；不可接受。
    HardLink,
}

/// 过滤 staging 的固定预算；任何一项为 0 都视为未设预算。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StagingBudget {
    /// 固定流式缓冲字节数。
    pub stream_buffer_bytes: u32,
    /// 允许的最大文件数。
    pub max_files: u64,
    /// 允许的最大总字节数。
    pub max_total_bytes: u64,
    /// 必须预留的空闲空间字节数。
    pub reserved_space_bytes: u64,
    /// 冻结期限毫秒数；超过即放弃本次封存。
    pub freeze_deadline_millis: u32,
}

impl StagingBudget {
    /// 本实现使用的固定预算。
    pub const FIXED: Self = Self {
        stream_buffer_bytes: 256 * 1024,
        max_files: 1_000_000,
        max_total_bytes: 512 * 1024 * 1024 * 1024,
        reserved_space_bytes: 2 * 1024 * 1024 * 1024,
        freeze_deadline_millis: 300_000,
    };

    /// 四项预算齐备才算真的有预算。
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.stream_buffer_bytes > 0
            && self.max_files > 0
            && self.max_total_bytes > 0
            && self.reserved_space_bytes > 0
            && self.freeze_deadline_millis > 0
    }
}

/// BackupWorker 唯一允许消费的只读传输源。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferSource {
    /// 传输源根目录；位于 Vault 与 `WORKBENCH_DATA` 之外。
    pub root: PathBuf,
    /// 产生方式。
    pub kind: TransferSourceKind,
    /// 物化方式。
    pub materialization: StagingMaterialization,
    /// 明确期限；不晚于最短来源保留期，避免形成隐藏恢复点。
    pub expires_at: InstantMillis,
    /// 固定预算。
    pub budget: StagingBudget,
}

impl TransferSource {
    /// 校验传输源本身；不访问文件系统，只检查已声明的事实。
    pub fn validate(
        &self,
        vault_root: &std::path::Path,
        workbench_data_root: &std::path::Path,
        shortest_source_retention: InstantMillis,
    ) -> Result<(), SealError> {
        if self.materialization == StagingMaterialization::HardLink {
            return Err(SealError::HardLinkIsNotImmutable);
        }
        if !self.budget.is_complete() {
            return Err(SealError::StagingBudgetMissing);
        }
        if self.root.starts_with(vault_root) || self.root.starts_with(workbench_data_root) {
            return Err(SealError::TransferSourceInsideProtectedRoot);
        }
        if self.expires_at > shortest_source_retention {
            return Err(SealError::TransferSourceOutlivesRetention);
        }
        Ok(())
    }
}

/// 封存票据：跨 owner 屏障成功并原子封存后才产生。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SealTicket {
    /// 本次 checkpoint 标识。
    pub checkpoint_id: CheckpointId,
    /// 六个 owner 的 present/absent 清单。
    pub roster: OwnerRoster,
    /// 已装配 owner 各自导出的已验证 checkpoint。
    pub owners: Vec<SealedOwnerCheckpoint>,
    /// 只读传输源。
    pub transfer_source: TransferSource,
    /// 内容截止时刻；恢复点年龄从此刻起算。
    pub content_cutoff: InstantMillis,
}

impl SealTicket {
    /// 取回某个 owner 的已验证 checkpoint。
    #[must_use]
    pub fn owner(&self, owner: CheckpointOwner) -> Option<&SealedOwnerCheckpoint> {
        self.owners.iter().find(|sealed| sealed.owner == owner)
    }
}

/// 封存边界的失败模式。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SealError {
    /// 清单缺项、乱序或重复，不能逐项标记 present/absent。
    RosterIncomplete,
    /// 对活动文件建硬链接不构成不可变视图。
    HardLinkIsNotImmutable,
    /// 有排除项却没有固定预算。
    StagingBudgetMissing,
    /// 传输源落在 Vault 或 `WORKBENCH_DATA` 之内。
    TransferSourceInsideProtectedRoot,
    /// 传输源期限晚于最短来源保留期，会形成隐藏恢复点。
    TransferSourceOutlivesRetention,
}

impl std::fmt::Display for SealError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::RosterIncomplete => "owner 清单不完整或顺序错误",
            Self::HardLinkIsNotImmutable => "硬链接活动文件不是不可变视图",
            Self::StagingBudgetMissing => "过滤 staging 缺少固定预算",
            Self::TransferSourceInsideProtectedRoot => "传输源位于 Vault 或 WORKBENCH_DATA 之内",
            Self::TransferSourceOutlivesRetention => "传输源期限晚于最短来源保留期",
        })
    }
}

impl std::error::Error for SealError {}
