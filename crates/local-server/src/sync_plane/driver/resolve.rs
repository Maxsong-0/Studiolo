//! 第 9 步：引导期（MirrorRemote）冲突预解决（ADR-025 D6）——镜像为准。
//!
//! canonical 版本先保全到 `ops/bridge-conflicts/`，再把 canonical 路径 CAS 到镜像版本；
//! 镜像缺失则条件删除（走文件操作协议，进墓碑）。删除仍受同一熔断判据。
//!
//! 幂等键与副本路径一一对应（评审 2026-09-28 P1-2）：键＝`conflict/<link>/<冲突身份>/<首次戳>/{preserve,resolve}`，
//! 冲突身份含两侧版本摘要（[`conflict_key`]），首次戳由端口先写进 `plane-state.json`（与 Bidirectional
//! 保全共用 `conflict_stamps`）。中途失败的下一轮沿用同一戳、同一路径、同一键续做，不会拿新分钟戳的
//! 副本路径去撞旧键；任一侧版本变了就是另一个冲突、另一组键。

use super::super::witness::seam_error;
use super::super::{PlaneError, naming};
use super::content::{MirrorWriteError, RoundContext, preserve_canonical, write_from_mirror};
use super::deletion_fuse_blocked;
use super::evidence::{hex, revision_from_digest};
use super::preserve::{conflict_key, preserved};
use crate::coordinator::sync_bridge::{BridgePrecondition, BridgeSeamError};
use std::collections::BTreeMap;
use studiolo_sync_bridge::{Decision, Presence};

/// 按「镜像为准」预解决本轮的冲突（按路径序至多 `MAX_CONFLICT_STAMPS` 条）；返回是否改动过
/// canonical（改过就必须重观察）。`stamps` 是端口已落盘的首次戳。
///
/// # Errors
/// 预解决删除越过熔断返回 `Fused`（一条也不解决）；冲突没有戳返回 `Storage`；保全或 CAS 失败返回对应分类。
pub fn preresolve_conflicts(
    context: &RoundContext<'_>,
    decision: &Decision,
    stamps: &BTreeMap<String, String>,
) -> Result<bool, PlaneError> {
    let (conflicts, live) = match decision {
        Decision::Ready(plan) => (plan.conflicts(), plan.stats().baseline_live_paths),
        Decision::FuseBlocked(blocked) => (
            blocked.conflicts.as_slice(),
            blocked.stats.baseline_live_paths,
        ),
    };
    if conflicts.is_empty() {
        return Ok(false);
    }
    // 先算整批删除：镜像缺失而 canonical 存在的冲突都是删除，越熔断则整批不解决。
    let deletions = conflicts
        .iter()
        .filter(|conflict| {
            matches!(
                (&conflict.observed.canonical, &conflict.observed.mirror),
                (Presence::Present(_), Presence::Missing)
            )
        })
        .count() as u64;
    if deletions > 0 && deletion_fuse_blocked(deletions, live) {
        return Err(PlaneError::Fused);
    }
    let workbench = context.plane.workbench();
    let link_id = context.binding.link_id;
    let mut changed = false;
    for conflict in preserved(decision) {
        let identity = conflict_key(&link_id, conflict);
        let stamp = stamps.get(&identity).ok_or(PlaneError::Storage)?;
        let base = format!("conflict/{}/{identity}/{stamp}", hex(&link_id));
        let resolve = format!("{base}/resolve");
        let outcome = match (&conflict.observed.canonical, &conflict.observed.mirror) {
            (Presence::Missing, Presence::Missing) => continue,
            (Presence::Missing, Presence::Present(mirror)) => write_from_mirror(
                context,
                &conflict.path,
                mirror,
                BridgePrecondition::MustNotExist,
                &resolve,
            )
            .map(|_| ())
            .map_err(MirrorWriteError::seam),
            (Presence::Present(canonical), mirror_side) => {
                let preserve = format!("{base}/preserve");
                let directory = naming::CONFLICTS_DIR;
                preserve_canonical(
                    context,
                    directory,
                    &conflict.path,
                    canonical,
                    stamp,
                    &preserve,
                )?;
                let expected = revision_from_digest(&canonical.revision);
                match mirror_side {
                    Presence::Present(mirror) => write_from_mirror(
                        context,
                        &conflict.path,
                        mirror,
                        BridgePrecondition::Match(expected),
                        &resolve,
                    )
                    .map(|_| ())
                    .map_err(MirrorWriteError::seam),
                    Presence::Missing => workbench
                        .bridge_delete(
                            &conflict.path,
                            expected,
                            &resolve,
                            &hex(&context.binding.scope_hash),
                            context.binding.scope_epoch,
                        )
                        .map(|_| ()),
                }
            }
        };
        match outcome {
            Ok(()) => changed = true,
            // 权威 CAS 拒绝：canonical 在观察之后已变，本轮到此为止，重观察后再规划。
            Err(BridgeSeamError::Rejected) => return Ok(true),
            Err(error) => return Err(seam_error(error)),
        }
    }
    Ok(changed)
}
