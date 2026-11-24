//! 冻结之前，先等人侧自己空下来（BUG-056）。
//!
//! 人侧的完整操作有长有短：一次登录几毫秒，一轮同步平时十几秒，外网不通时一分多钟。排空要等它们都做完，
//! 可冻结的排空有预算、还先挡住了接入方：169 上一轮同步卡了 61 秒，封存等满 30 秒放弃，少一份恢复点，
//! 接入方陪着等了 30 秒。所以先在什么都没挡住的时候等，等到了再去排空；等的时候新操作照样进得来。
//!
//! 只等人侧：接入方的完整操作是一次请求，自己有界；而且人侧不等它，它排空时人侧照常。
//! 空下来到真排空之间仍有一小段：撞上了照旧在排空那一步等、用冻结预算，不在这里重试。
use super::*;
use studiolo_audit_ledger::OPERATION_RETRY;

/// 等的结果：空下来了，或者被叫停了（进程在关停）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HumanSide {
    /// 看的那一刻没有完整操作在做。
    Idle,
    /// 叫停了，没等到。
    Stopped,
}

impl CheckpointOwners {
    /// 等到人侧此刻没有完整操作在做；到 `until` 还等不到就失败。
    ///
    /// 隔 [`OPERATION_RETRY`] 看一次，和排空自己的重试一样勤，它看得到的空当这里也看得到。
    /// 等的时候不留租约：人侧忙着，试拿不到，什么都没占；看到空的那一眼拿到独占许可、当场还掉，那一瞬（几十微秒）
    /// 别人试取会扑空，各处本来就会重试或跳过一轮。`stop_within` 睡不超过给它的时长，返回真就是叫停；
    /// 每次看之前都先问它，叫停了就不再看。
    pub fn await_human_idle(
        &self,
        until: Instant,
        stop_within: impl Fn(Duration) -> bool,
    ) -> Result<HumanSide> {
        let mut span = Duration::ZERO;
        loop {
            if stop_within(span) {
                return Ok(HumanSide::Stopped);
            }
            if self.audit.idle().map_err(|_| unavailable(Owner::Audit))? {
                return Ok(HumanSide::Idle);
            }
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(CheckpointError::NotDrained {
                    owner: Owner::Audit,
                });
            }
            span = OPERATION_RETRY.min(left);
        }
    }

    /// 冻结自己开头也等一次，用冻结预算：补记登录库之后、挡住接入方之前。生产者在算冻结预算之前已经等过，
    /// 这里补的是那之后又起的慢操作。
    pub(super) fn settle_human(&self, deadline: Instant) -> Result<()> {
        self.await_human_idle(deadline, |span| {
            std::thread::sleep(span);
            false
        })
        .map(drop)
    }
}

/// 排空没做成：到了截止时间的是没排空，别的是账本本身不可用。以前都报「不可用」，
/// 169 那次失败的日志就看不出是在等别人做完（BUG-056）。两种错在账本那层归成一类，只能看时刻分：
/// 账本正好在截止那一刻坏掉会被说成没排空，只影响这一行日志，账本里记的原因不变。
pub(super) fn undrained(owner: Owner, deadline: Instant) -> CheckpointError {
    if Instant::now() >= deadline {
        CheckpointError::NotDrained { owner }
    } else {
        unavailable(owner)
    }
}
