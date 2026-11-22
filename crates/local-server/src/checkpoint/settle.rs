//! 冻结之前，先把登录库自己补得掉的待办补完（BUG-044）。
//!
//! 登录库冻结时要求：待发件箱里没有没补记的审计事件、拒绝计数都交付了、没有做到一半的保留期计划、
//! 没有可以释放的失效 pending。别的库各有周期任务替它们清（黑板 15 秒一轮的巡检、同步平面的固定轮次、
//! 写者每次操作之后），登录库没有：补记只在登录、吊销时顺带做。会话过了空闲期、之后没人再登录，
//! 那条「会话过期」就一直留着，封存每一轮都在登录库这一步失败。谁要求清空，谁先把它清掉。
//!
//! 这里只调登录库自己的可信维护入口，不重放动作、不发放权限。补完之后到登录库冻结之间仍有一小段
//! 窗口：普通的会话检查不取完整操作许可就能让新的过期、拒绝事件入队。撞上了，这一轮照旧在登录库冻结时
//! 失败、下一轮再试，不在这里重试。
use super::*;
use studiolo_auth::RetentionWork;

/// 最多补几轮。一轮最多补 256 条会话事件、回收 16 个终态会话；平时一轮就清空，
/// 积压多的时候不把冻结预算全花在这里，剩下的留给下一次封存。
const ROUNDS: usize = 4;

impl CheckpointOwners {
    /// 必须在排空之前调：补记要取完整操作许可，排空之后取不到。
    pub(super) fn settle_auth(&self, deadline: Instant) -> Result<()> {
        for _ in 0..ROUNDS {
            check_deadline(deadline, Owner::Auth)?;
            let delivered = match &self.factors {
                // 含两步验证、恢复码的待发审计和失效 pending 的释放，以及会话事件。
                Some(factors) => factors.flush_audit(),
                None => self.sessions.flush_audit(256),
            }
            .map_err(|_| unavailable(Owner::Auth))?;
            // 做到一半留下的保留期计划只有这个入口能接着做完；没有可回收的终态会话时它什么都不写。
            let retention = self
                .sessions
                .maintain_retention()
                .map_err(|_| unavailable(Owner::Auth))?;
            if delivered == 0
                && retention.reclaimed_sessions == 0
                && retention.remaining != RetentionWork::More
            {
                break;
            }
        }
        Ok(())
    }
}
