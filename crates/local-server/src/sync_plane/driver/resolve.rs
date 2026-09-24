//! 第 9 步：引导期（MirrorRemote）冲突预解决（ADR-025 D6）——镜像为准。
//!
//! canonical 版本先保全到 `ops/bridge-conflicts/`，再把 canonical 路径 CAS 到镜像版本；
//! 镜像缺失则条件删除（走文件操作协议，进墓碑）。删除仍受同一熔断判据。

use super::super::witness::seam_error;
use super::super::{PlaneError, naming};
use super::content::{RoundContext, preserve_canonical, write_from_mirror};
use super::deletion_fuse_blocked;
use super::evidence::{hex, sha256};
use crate::coordinator::sync_bridge::{BridgePrecondition, BridgeSeamError};
use studiolo_sync_bridge::{Conflict, Decision, PlanKey, Presence};

/// 按「镜像为准」预解决全部冲突；返回是否改动过 canonical（改过就必须重观察）。
///
/// # Errors
/// 预解决删除越过熔断返回 `Fused`（一条也不解决）；保全或 CAS 失败返回对应分类。
pub fn preresolve_conflicts(
    context: &RoundContext<'_>,
    decision: &Decision,
) -> Result<bool, PlaneError> {
    let (key, conflicts, live): (&PlanKey, &[Conflict], u64) = match decision {
        Decision::Ready(plan) => (
            plan.key(),
            plan.conflicts(),
            plan.stats().baseline_live_paths,
        ),
        Decision::FuseBlocked(blocked) => (
            &blocked.key,
            &blocked.conflicts,
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
    let mut changed = false;
    for conflict in conflicts {
        let base = format!(
            "conflict/{}/{}/{}",
            hex(&key.link_id),
            key.sequence,
            hex(&sha256(conflict.path.as_str().as_bytes()))
        );
        let outcome = match (&conflict.observed.canonical, &conflict.observed.mirror) {
            (Presence::Missing, Presence::Missing) => continue,
            (Presence::Missing, Presence::Present(mirror)) => write_from_mirror(
                context,
                &conflict.path,
                mirror,
                BridgePrecondition::MustNotExist,
                &base,
            )
            .map(|_| ()),
            (Presence::Present(canonical), mirror_side) => {
                preserve_canonical(
                    context,
                    naming::CONFLICTS_DIR,
                    &conflict.path,
                    canonical,
                    &format!("{base}/preserve"),
                )?;
                let expected = context
                    .revisions
                    .borrow()
                    .restore(&canonical.revision)
                    .ok_or(PlaneError::Observation)?;
                match mirror_side {
                    Presence::Present(mirror) => write_from_mirror(
                        context,
                        &conflict.path,
                        mirror,
                        BridgePrecondition::Match(expected),
                        &base,
                    )
                    .map(|_| ()),
                    Presence::Missing => workbench
                        .bridge_delete(
                            &conflict.path,
                            expected,
                            &base,
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
