//! `history/meta-v1` 与 `maintenance-v1` 绑定记录的等价定义。
//!
//! 这两份记录由各自的 Module 私有持有（`pub(super)` / 私有结构），
//! 恢复不能借用它们的类型，只能用**字段名与 `deny_unknown_fields` 完全一致**的
//! 等价定义读写同一份字节。`restore/tests/drift.rs` 会用真实初始化出来的字节
//! 反序列化这两个定义，一旦上游改字段就会立刻变红，而不是悄悄写坏绑定。

use crate::VaultUuid;
use crate::writer::Identity;
use serde::{Deserialize, Serialize};

/// `history/meta-v1` 的目录名。
pub(super) const HISTORY_META: &str = "meta-v1";
/// `maintenance-v1` 的目录名。
pub(super) const MAINTENANCE: &str = "maintenance-v1";
/// `maintenance-v1` 半初始化时留下的目录名。
pub(super) const MAINTENANCE_PREPARING: &str = "maintenance-v1.preparing";
/// 两处主 broker-binding 使用同一个文件名。
pub(super) const BROKER_BINDING: &str = "broker-binding.json";
/// 版本化元数据目录里的绑定文件名。
pub(super) const BINDING: &str = "binding.json";
/// canonical 侧的私有控制目录。
pub(super) const CONTROL: &str = ".workbench";
/// 写者持久记录。
pub(super) const WRITER_STATE: &str = "writer-state.json";

/// `history/meta-v1/binding.json` 的等价定义。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct HistoryBinding {
    pub(super) version: u32,
    pub(super) pair_id: VaultUuid,
    pub(super) vault_uuid: VaultUuid,
    pub(super) root: Identity,
    pub(super) requests: Identity,
    pub(super) catalogs: Identity,
    pub(super) outbox: Identity,
}

/// `maintenance-v1/binding.json` 的等价定义。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MaintenanceBinding {
    pub(super) version: u32,
    pub(super) pair_id: VaultUuid,
    pub(super) vault_uuid: VaultUuid,
    pub(super) root: Identity,
}
