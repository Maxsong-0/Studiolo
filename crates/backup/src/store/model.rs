//! 持久状态的纯 DTO 与存储 Interface；这里没有任何 IO。

use crate::checkpoint::manifest::CheckpointManifest;
use crate::worker::recovery_point::RecoveryPointStamps;
use crate::worker::state::JobRecord;
use serde::{Deserialize, Serialize};

/// 持久状态 schema 版本；未知版本失败关闭。
pub const STATE_SCHEMA_VERSION: u32 = 1;

/// `backup-state.json` 的完整内容。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersistedState {
    /// schema 版本。
    pub schema_version: u32,
    /// 下一个作业序号；单调递增，不复用历史空号。
    pub next_sequence: u64,
    /// 尚未进入终态或仍待补记的作业。
    pub jobs: Vec<JobRecord>,
    /// 最近一个普通调度恢复点。
    pub last_scheduled: Option<RecoveryPointStamps>,
    /// 最近一个每日追加／不可变恢复点。
    pub last_daily_immutable: Option<RecoveryPointStamps>,
    /// 首次完整 seed 是否已校验通过。
    pub seed_verified: bool,
}

impl PersistedState {
    /// 全新存储的初始状态。
    #[must_use]
    pub fn initial() -> Self {
        Self {
            schema_version: STATE_SCHEMA_VERSION,
            next_sequence: 1,
            jobs: Vec::new(),
            last_scheduled: None,
            last_daily_immutable: None,
            seed_verified: false,
        }
    }

    /// 校验一份读回的状态。
    pub fn validate(&self) -> Result<(), StoreError> {
        if self.schema_version != STATE_SCHEMA_VERSION {
            return Err(StoreError::UnsupportedSchema);
        }
        if self.next_sequence == 0 {
            return Err(StoreError::CorruptStore);
        }
        for job in &self.jobs {
            if job.key.sequence.get() >= self.next_sequence {
                return Err(StoreError::CorruptStore);
            }
        }
        Ok(())
    }
}

/// 持久存储 Interface；实现落在 `store::storage`。
pub trait BackupStateStore {
    /// 读取持久状态；存储为空返回 `None`。
    fn load_state(&self) -> Result<Option<PersistedState>, StoreError>;

    /// 原子替换持久状态。
    fn save_state(&self, state: &PersistedState) -> Result<(), StoreError>;

    /// 原子提交最终清单；提交成功才可能存在可恢复点。
    fn commit_final_manifest(&self, manifest: &CheckpointManifest) -> Result<(), StoreError>;

    /// 读取已提交的最终清单。
    fn load_final_manifest(&self) -> Result<Option<CheckpointManifest>, StoreError>;
}

/// 存储层失败模式。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoreError {
    /// 私有目录、权限或对象身份不满足约束。
    UnsafeStorage,
    /// 固定文件不存在，需要显式初始化。
    MissingStore,
    /// 存储已存在，不能重复初始化。
    AlreadyExists,
    /// 内容不是完整合法 JSON，或字段不自洽。
    CorruptStore,
    /// schema 版本未知。
    UnsupportedSchema,
    /// 单个文件超过固定上限。
    LimitExceeded,
    /// 磁盘空间不足。
    OutOfSpace,
    /// 底层调用失败。
    Unavailable,
    /// 当前平台未实现安全的私有文件约束。
    UnsupportedPlatform,
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::UnsafeStorage => "备份状态目录不满足私有存储约束",
            Self::MissingStore => "备份状态存储不存在；需要显式初始化",
            Self::AlreadyExists => "备份状态存储已存在",
            Self::CorruptStore => "备份状态内容不完整或不自洽",
            Self::UnsupportedSchema => "备份状态 schema 版本未知",
            Self::LimitExceeded => "备份状态超过固定字节上限",
            Self::OutOfSpace => "磁盘空间不足",
            Self::Unavailable => "备份状态存储不可用",
            Self::UnsupportedPlatform => "当前平台不支持安全的私有备份状态存储",
        })
    }
}

impl std::error::Error for StoreError {}
