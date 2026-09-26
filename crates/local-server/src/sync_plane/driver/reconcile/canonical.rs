//! 重开核对的 canonical 侧：先把冻结前置还原成原 `Revision`，再经执行器同一份
//! `PlaneJobBackend::canonical_verdict` 重建首次请求走接缝，由接缝按原键查重放。
//!
//! 原 `Revision` 只活在观察它的那一轮（Bridge 证据里只有它的摘要）；第 10 步还没观察，所以这里经接缝
//! 有界读一次当前 canonical：摘要仍等于冻结前置才登记。已不是前置（已安装或被人改过）就还原不出
//! 原请求，只能保持未知——绝不拿相同内容或当前版本拼一个不同的请求去碰原键。

use super::super::content::RoundContext;
use super::super::evidence::revision_digest;
use crate::sync_plane::job_backend::{PlaneJobBackend, SeamVerdict};
use studiolo_core::VaultPath;
use studiolo_sync_bridge::executor::IssuedJob;
use studiolo_sync_bridge::{Action, Change, FileVersion};
use studiolo_sync_transport::owned_run::QuiescentMirror;

/// 按原键重放一条 canonical 未知动作；前置还原不出即 `Unknown`（不调用接缝）。
pub(super) fn replay(
    context: &RoundContext<'_>,
    ticket: &QuiescentMirror,
    job: &IssuedJob,
    action: &Action,
) -> SeamVerdict {
    if let Change::Replace {
        target_expected, ..
    }
    | Change::Delete { target_expected } = &action.change
        && !restore_precondition(context, &action.key.path, target_expected)
    {
        return SeamVerdict::Unknown;
    }
    let mut backend = PlaneJobBackend::new(context, ticket, context.binding.clone());
    backend
        .canonical_verdict(job, action)
        .unwrap_or(SeamVerdict::Unknown)
}

/// 本轮 `Revision` 表里没有冻结前置时，经接缝读当前 canonical（上限＝前置字节数）；
/// 其 revision 摘要等于冻结前置才登记原 `Revision` 并返回真。
fn restore_precondition(
    context: &RoundContext<'_>,
    path: &VaultPath,
    expected: &FileVersion,
) -> bool {
    if context
        .revisions
        .borrow()
        .restore(&expected.revision)
        .is_some()
    {
        return true;
    }
    let Ok(read) = context.plane.workbench().bridge_read(path, expected.bytes) else {
        return false;
    };
    if revision_digest(read.revision) != expected.revision {
        return false;
    }
    context.revisions.borrow_mut().remember(read.revision);
    true
}
