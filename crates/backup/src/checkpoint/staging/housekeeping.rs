//! 先完整核对所有登记，再按原身份回收中断副本与过期封存；没有隐式初始化或按名称删除。
//! 删登记目录先原子改名、身份记录最后删；上一轮删到一半的，按改名时写进名字的原 inode 认出来接着删。
mod plan;
use super::{
    ReaderIdentity, Result, StagingError,
    budget::Budget,
    check_deadline,
    io::{Directory, JournalFile},
    journal,
};
use std::{
    path::Path,
    time::{Duration, Instant},
};
use studiolo_core::checkpoint::InstantMillis;

/// 一次受控回收的非正文结果；下一期限供生产服务安排唤醒。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CleanupSummary {
    /// 已确认回收的中断尝试数。
    pub interrupted: u32,
    /// 已确认回收的过期封存数。
    pub expired: u32,
    /// 尚未到期且仍被保留的封存数。
    pub retained: u32,
    /// 上一轮删登记目录删到一半（已改成 `removing-…`）、这一轮接着删完的数目。
    pub resumed: u32,
    /// 最近的尚未到期时刻；调用方必须安排再次回收。
    pub next_expiry: Option<InstantMillis>,
}
/// 使用固定已存在的生产者租约；本 API 不创建父目录、锁文件或任何业务状态。
/// 只允许与 create/create_for_reader 相同的生产者及读取组部署配置。
/// 无登记的旧副本、记录损坏或身份变化返回错误，不猜测删除。
pub fn reclaim_staging(
    parent_path: &Path,
    reader_gid: Option<u32>,
    deadline: Instant,
) -> Result<CleanupSummary> {
    let deadline = deadline.min(Instant::now() + Duration::from_secs(30));
    reclaim(parent_path, reader_gid, &mut Budget::until(deadline))
}
/// 回收本体；删除的每一步记在给定的预算里（私有测试用它停在任意一步）。
pub(super) fn reclaim(
    parent_path: &Path,
    reader_gid: Option<u32>,
    budget: &mut Budget,
) -> Result<CleanupSummary> {
    budget.check()?;
    let deadline = budget.deadline();
    let reader = reader_gid.map(ReaderIdentity::producer).transpose()?;
    let parent = match reader {
        Some(id) => Directory::publisher(parent_path, id)?,
        None => Directory::open(parent_path, true)?,
    };
    let lease = parent.acquire_existing_lease()?;
    let started = journal::now()?;
    let plans = plan::read_all(&parent, reader, started, deadline)?;
    let mut result = CleanupSummary::default();
    for plan in plans {
        budget.check()?;
        parent.check_path(parent_path)?;
        lease.check(&parent)?;
        plan.check(&parent)?;
        let now = journal::now()?;
        if now < started {
            return Err(StagingError::Expired);
        }
        if let Some(expiry) = plan.expires_at
            && expiry > now
        {
            result.retained += 1;
            result.next_expiry = Some(result.next_expiry.map_or(expiry, |prior| prior.min(expiry)));
            continue;
        }
        for (name, root) in &plan.roots {
            plan.check(&parent)?;
            root.discard_within(&parent, name, reader, budget)?;
        }
        parent.check_path(parent_path)?;
        lease.check(&parent)?;
        plan.check(&parent)?;
        let last = JournalFile::Identity.names().0;
        plan.marker
            .retire(&parent, &plan.name, &plan.removing, last, budget)?;
        if plan.resumed {
            result.resumed += 1;
        } else if plan.expires_at.is_some() {
            result.expired += 1;
        } else {
            result.interrupted += 1;
        }
    }
    parent.check_path(parent_path)?;
    lease.check(&parent)?;
    budget.check()?;
    if journal::now()? < started {
        return Err(StagingError::Expired);
    }
    Ok(result)
}
