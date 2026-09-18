//! 恢复 lineage：新目录从哪个 checkpoint、哪一对旧物理身份来的。
//!
//! lineage 先以 `Rebinding` 落盘，重绑定全部完成之后才改写为 `Completed`。
//! 读到 `Rebinding` 就说明上一次恢复中途断了，该新目录必须人工复核。

use crate::VaultUuid;
use crate::writer::Identity;
use serde::{Deserialize, Serialize};

/// lineage 目录名，位于新的 data 根下。
pub(super) const LINEAGE_DIR: &str = "restore-v1";
/// lineage 文件相对 data 根的路径。
pub(super) const LINEAGE_PATH: &str = "restore-v1/lineage.json";

/// 一处目录的物理身份。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PhysicalId {
    /// 设备号。
    pub dev: u64,
    /// inode 号。
    pub ino: u64,
}

impl From<Identity> for PhysicalId {
    fn from(identity: Identity) -> Self {
        Self {
            dev: identity.dev,
            ino: identity.ino,
        }
    }
}

/// lineage 记录的阶段。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LineageState {
    /// 物理重绑定进行中；此时新目录必须保持不可服务。
    Rebinding,
    /// 全部重绑定完成并已核验。
    Completed,
}

/// 新目录的恢复 lineage。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RestoreLineage {
    /// 记录格式版本。
    pub format_version: u32,
    /// 当前阶段。
    pub state: LineageState,
    /// 保留下来的已验证 Vault 身份。
    pub vault_uuid: VaultUuid,
    /// 保留下来的逻辑配对标识。
    pub pair_id: VaultUuid,
    /// 本次恢复所基于的 checkpoint 标识。
    pub checkpoint_id: String,
    /// 恢复前 canonical 根的物理身份。
    pub previous_canonical: PhysicalId,
    /// 恢复前 data 根的物理身份。
    pub previous_data: PhysicalId,
    /// 重绑定后 canonical 根的物理身份。
    pub restored_canonical: Option<PhysicalId>,
    /// 重绑定后 data 根的物理身份。
    pub restored_data: Option<PhysicalId>,
    /// 上一次恢复的 lineage 标识，形成可追溯的链。
    pub parent_lineage: Option<String>,
}

impl RestoreLineage {
    /// 本记录格式版本。
    pub(super) const FORMAT_VERSION: u32 = 1;

    /// 该 lineage 是否代表一次已完成、可服务的恢复。
    #[must_use]
    pub fn is_completed(&self) -> bool {
        self.state == LineageState::Completed
            && self.restored_canonical.is_some()
            && self.restored_data.is_some()
    }
}
