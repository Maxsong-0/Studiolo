//! 重开核对的 canonical 侧：经执行器同一份 `PlaneJobBackend::canonical_verdict` 重建首次请求走接缝，
//! 由接缝按原键查重放。
//!
//! 冻结前置的原 `Revision` 就是 Bridge 证据里的 32 字节原值（`driver::evidence::revision_from_digest`），
//! 任何一轮都能还原：目标已安装、被人改过或首次回执丢失都不妨碍按原键核对——重建的请求与首次逐字段相同，
//! 有原回执照收，接缝绝不二次安装；没有原回执时由接缝按同一前置首次安装或 CAS 拒绝。

use super::super::content::RoundContext;
use crate::sync_plane::job_backend::{PlaneJobBackend, SeamVerdict};
use studiolo_sync_bridge::Action;
use studiolo_sync_bridge::executor::IssuedJob;

/// 按原键重放一条 canonical 未知动作。
pub(super) fn replay(context: &RoundContext<'_>, job: &IssuedJob, action: &Action) -> SeamVerdict {
    PlaneJobBackend::new(context, context.binding.clone()).canonical_verdict(job, action)
}
