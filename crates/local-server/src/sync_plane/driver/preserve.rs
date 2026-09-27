//! 第 9 步（Bidirectional，P6 语义）：冲突不自动解决，只把双方现存版本各保全一份到 canonical 的
//! `ops/bridge-conflicts/`（canonical 版本用设备名 `studiolo-canonical`，镜像版本用审批文档的
//! `device_name`），然后照常执行——执行器对冲突路径不动、闭合时不推进基线，等人选边。
//! 纯组合：canonical 读写只经 `content.rs` 的公开函数与接缝，镜像读先复验静止票据；本文件没有 IO 记号。
//!
//! 幂等分两段：[`assign_stamps`] 给冲突配首次保全的 UTC 分钟戳（纯函数，端口随即把戳写进
//! `plane-state.json` 的 `conflict_stamps`），[`preserve_conflicts`] 再按已落盘的戳写副本。戳先落盘、
//! 后写副本，所以同一冲突跨轮总落在同一副本路径、同一幂等键上：有原回执即命中，副本已在而回执
//! 不在即 `Rejected`（视为已保全），绝不覆盖、不自动选边。幂等键带上戳：冲突消失后原样重现
//! （旧戳已回收）会按新戳另存一份，而不是拿新路径撞旧键、被接缝判成未知而卡住每一轮。
//! 冲突超过 [`MAX_CONFLICT_STAMPS`] 条时只保全按路径序的前 512 条，其余等前面的解决后再轮到。

use super::super::recovery::MAX_CONFLICT_STAMPS;
use super::super::witness::seam_error;
use super::super::{PlaneError, naming};
use super::content::{self, RoundContext};
use super::evidence::{hex, sha256};
use crate::coordinator::sync_bridge::{BridgePrecondition, BridgeSeamError};
use std::collections::{BTreeMap, BTreeSet};
use studiolo_core::VaultPath;
use studiolo_sync_bridge::{Conflict, Decision, Presence};

/// 冲突身份摘要的域分隔前缀。
const DOMAIN: &[u8] = b"studiolo.sync-plane.conflict.v1\0";

/// 冲突身份：`hex(sha256(域 ‖ link_id ‖ 路径 ‖ canonical 摘要或 32 个 0 ‖ 镜像摘要或 32 个 0))`。
/// 同一冲突（路径与两侧版本都不变）跨轮稳定；任一侧版本变了就是另一个冲突。
#[must_use]
pub fn conflict_key(link_id: &[u8; 16], conflict: &Conflict) -> String {
    let side = |presence: &Presence| match presence {
        Presence::Present(version) => version.sha256,
        Presence::Missing => [0u8; 32],
    };
    let path = conflict.path.as_str().as_bytes();
    let mut identity = Vec::with_capacity(DOMAIN.len() + 16 + path.len() + 64);
    identity.extend_from_slice(DOMAIN);
    identity.extend_from_slice(link_id);
    identity.extend_from_slice(path);
    identity.extend_from_slice(&side(&conflict.observed.canonical));
    identity.extend_from_slice(&side(&conflict.observed.mirror));
    hex(&sha256(&identity))
}

/// 第一段（纯）：给本轮要保全的冲突配首次保全戳——已有戳的沿用，没有的取 `now_unix` 的 UTC
/// 分钟戳；收尾只留本轮这些冲突的戳（已解决、已变或排在 512 条之外的冲突的戳回收）。
/// 返回新配戳的冲突数，即本轮新保全的冲突数。
///
/// # Errors
/// 需要新戳而 `now_unix` 格式化不出分钟戳（早于纪元）返回 `Unavailable`。
pub fn assign_stamps(
    link_id: &[u8; 16],
    decision: &Decision,
    stamps: &mut BTreeMap<String, String>,
    now_unix: i64,
) -> Result<u64, PlaneError> {
    let current: BTreeSet<String> = preserved(decision)
        .into_iter()
        .map(|conflict| conflict_key(link_id, conflict))
        .collect();
    let fresh: Vec<&String> = current
        .iter()
        .filter(|key| !stamps.contains_key(*key))
        .collect();
    if !fresh.is_empty() {
        let stamp = naming::utc_minute_stamp(now_unix).ok_or(PlaneError::Unavailable)?;
        for key in &fresh {
            stamps.insert((*key).clone(), stamp.clone());
        }
    }
    stamps.retain(|key, _| current.contains(key));
    Ok(u64::try_from(fresh.len()).unwrap_or(u64::MAX))
}

/// 第二段：按已落盘的戳把本轮要保全的冲突逐条写成副本——canonical 侧存在就保全 canonical 版本
/// （设备名 `studiolo-canonical`），镜像侧存在就保全镜像版本（设备名 `device`，正文读前复验静止）；
/// 条件都是 `MustNotExist`，幂等键 `conflict/<hex link_id>/<冲突身份>/<戳>/{canonical,mirror}`。
///
/// # Errors
/// 冲突没有戳返回 `Storage`（端口必须先配戳落盘）；副本命名失败返回 `Unavailable`；正文读不出或
/// 已不是冻结版本按 `content.rs` 分类返回；写入 `NotAttempted`／`Unavailable`／`Unknown`／`Audit`
/// 按接缝分类返回（本轮停），`Rejected`（副本已在、重放记录不在）视为已保全。
pub fn preserve_conflicts(
    context: &RoundContext<'_>,
    decision: &Decision,
    stamps: &BTreeMap<String, String>,
    device: &str,
) -> Result<(), PlaneError> {
    let link_id = context.binding.link_id;
    for conflict in preserved(decision) {
        let key = conflict_key(&link_id, conflict);
        let stamp = stamps.get(&key).ok_or(PlaneError::Storage)?;
        let base = format!("conflict/{}/{key}/{stamp}", hex(&link_id));
        if let Presence::Present(version) = &conflict.observed.canonical {
            let copy = copy_path(conflict, naming::CANONICAL_DEVICE, stamp)?;
            let bytes = content::read_canonical_bytes(context.plane, &conflict.path, version)?;
            create(context, &copy, bytes, &format!("{base}/canonical"))?;
        }
        if let Presence::Present(version) = &conflict.observed.mirror {
            let copy = copy_path(conflict, device, stamp)?;
            let bytes = content::read_mirror_bytes(context, &conflict.path, version)?;
            create(context, &copy, bytes, &format!("{base}/mirror"))?;
        }
    }
    Ok(())
}

/// 本轮要保全的冲突：规划结果或熔断冻结里的冲突，按路径序取前 [`MAX_CONFLICT_STAMPS`] 条。
fn preserved(decision: &Decision) -> Vec<&Conflict> {
    let conflicts = match decision {
        Decision::Ready(plan) => plan.conflicts(),
        Decision::FuseBlocked(blocked) => blocked.conflicts.as_slice(),
    };
    let ordered: BTreeMap<&VaultPath, &Conflict> = conflicts
        .iter()
        .map(|conflict| (&conflict.path, conflict))
        .collect();
    ordered.into_values().take(MAX_CONFLICT_STAMPS).collect()
}

/// `ops/bridge-conflicts/` 下的副本路径；设备名或结果路径不合法返回 `Unavailable`。
fn copy_path(conflict: &Conflict, device: &str, stamp: &str) -> Result<VaultPath, PlaneError> {
    naming::copy_path(naming::CONFLICTS_DIR, &conflict.path, device, stamp)
        .ok_or(PlaneError::Unavailable)
}

/// 把一侧正文条件创建成副本；副本已在（重放记录不在）即 `Rejected`，按已保全处理，不覆盖。
fn create(
    context: &RoundContext<'_>,
    copy: &VaultPath,
    bytes: Vec<u8>,
    key: &str,
) -> Result<(), PlaneError> {
    let precondition = BridgePrecondition::MustNotExist;
    match content::write_bytes(context.plane, context.stage, copy, bytes, precondition, key) {
        Ok(_) | Err(BridgeSeamError::Rejected) => Ok(()),
        Err(error) => Err(seam_error(error)),
    }
}
