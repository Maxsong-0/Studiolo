//! 刷新闸门状态机：至多一次在途派生加至多一次排队派生，其余请求一律合流。
//!
//! 这里只有纯状态转换：不创建线程、不加锁、不等待、不读时钟、不做 I/O。调用方
//! 自己提供互斥与唤醒手段（同步 `Condvar`、异步 `watch` 皆可），闸门只回答
//! 「谁去跑、跑到哪张票、结果如何」。这样取消一个等待者不会带走在途派生，
//! 也不会让 N 个并发请求各跑一遍全量派生。
//!
//! 排他性由 [`RefreshGate::claim`] 单点强制，而不是由「谁拿到 `Start`」强制：
//! 认领是唯一写入在途位的入口，所以即使多个请求同时拿到 `Start`，也只有一个
//! 能认领到派生，其余立刻读到 `None` 并转为等待。这样「运行者忘记继续认领」
//! 只会推迟排队中的那次派生，不会把闸门永久卡死——下一个请求会接手。

use crate::refresh_dto::{
    RefreshAdmission, RefreshClosed, RefreshGateStats, RefreshRun, RefreshTicket, RefreshWait,
};

/// 并发刷新请求的有界合流闸门。
///
/// 不变量：任一时刻至多一次派生在途、至多一次派生排队，因此任意数量的并发请求
/// 最多换来两次派生。排队位在请求到达时才占用，所以排队的那次派生一定开始于这些
/// 请求之后，合流不会把更旧的结果当成更新的结果交付。
#[derive(Debug)]
pub struct RefreshGate {
    requested: u64,
    finished: u64,
    pending: bool,
    reason: &'static str,
    /// 在途派生覆盖到的票号；`None` 表示当前没有派生在途。
    ///
    /// 每次认领都消耗一张新票，因此这个票号在历次认领之间严格递增，可直接充当
    /// 在途派生的身份，用来识别并忽略被判定为遗弃之后才迟到的结算。
    in_flight: Option<u64>,
    closed: bool,
    failure: Option<&'static str>,
    stats: RefreshGateStats,
}

impl Default for RefreshGate {
    fn default() -> Self {
        Self::new()
    }
}

impl RefreshGate {
    /// 新建闸门，票号从零开始，没有在途或排队的派生。
    #[must_use]
    pub fn new() -> Self {
        Self::resuming_from(0)
    }

    /// 从既有票号继续，用于进程内重建闸门而不复用旧票号。
    ///
    /// 已完成水位同时前移到该票号：重建之前的票在新闸门里没有等待者。
    #[must_use]
    pub fn resuming_from(ticket: u64) -> Self {
        Self {
            requested: ticket,
            finished: ticket,
            pending: false,
            reason: "idle",
            in_flight: None,
            closed: false,
            failure: None,
            stats: RefreshGateStats::default(),
        }
    }

    /// 受理一次刷新请求，返回票号与角色。
    ///
    /// 闸门空闲时本请求被要求去认领派生；已有派生在途时本请求只等待。无论哪种
    /// 角色，同一批未开始的请求共享同一张票，因此至多换来一次额外派生。票号触顶时
    /// 闸门主动关闭并拒绝，不回绕重用票号。
    pub fn request(&mut self, reason: &'static str) -> Result<RefreshAdmission, RefreshClosed> {
        if self.closed {
            return Err(RefreshClosed);
        }
        if !self.pending {
            let Some(next) = self.requested.checked_add(1) else {
                // 票号触顶：继续发号会让旧票与新票无法区分，只能停止受理。
                self.close();
                return Err(RefreshClosed);
            };
            self.requested = next;
            self.pending = true;
            self.reason = reason;
        }
        self.stats.admitted = self.stats.admitted.saturating_add(1);
        let ticket = RefreshTicket(self.requested);
        if self.in_flight.is_some() {
            return Ok(RefreshAdmission::Join(ticket));
        }
        Ok(RefreshAdmission::Start(ticket))
    }

    /// 认领排队中的派生；没有排队工作、已有派生在途或闸门已关闭时返回 `None`。
    ///
    /// 这是写入在途位的唯一入口，也是排他性的唯一强制点：并发调用里只有一个能
    /// 拿到 `Some`。拿到 `Start` 的调用方必须循环 `claim` 与 `settle` 直到 `claim`
    /// 返回 `None`；立刻返回 `None` 表示别的运行者已经接手，改为等待自己的票即可。
    ///
    /// 等待者每次被唤醒、读到自己的票仍是 `Running` 时，也应当再试一次 `claim`：
    /// 返回 `Some` 就由它接手。原运行者死亡并被判定为遗弃之后，正是靠这一步让
    /// 排队中的派生重新有人跑，而不必等到下一个新请求到来。
    pub fn claim(&mut self) -> Option<RefreshRun> {
        if self.closed || !self.pending || self.in_flight.is_some() {
            return None;
        }
        self.pending = false;
        self.in_flight = Some(self.requested);
        self.stats.runs_started = self.stats.runs_started.saturating_add(1);
        Some(RefreshRun {
            covers: self.requested,
            reason: self.reason,
        })
    }

    /// 结算一次派生。成功清除失败状态，失败记录理由但不锁死闸门。
    ///
    /// 已完成水位只前进不回退；失败之后闸门回到可再次请求的状态，恢复方式就是
    /// 下一次 `request` 重新跑一遍完整派生，没有需要人工清理的中间态。
    ///
    /// 已被 [`RefreshGate::fail_abandoned_run`] 判定为遗弃的派生如果又迟到结算，
    /// 它携带的票号不再等于在途票号，本次结算整体忽略：既不前移完成水位，也不
    /// 覆盖已经写下的失败，避免「死而复生」的运行者抹掉等待者已经读到的结论。
    pub fn settle(&mut self, run: RefreshRun, outcome: Result<(), &'static str>) {
        if self.in_flight != Some(run.covers) {
            self.stats.settles_ignored = self.stats.settles_ignored.saturating_add(1);
            return;
        }
        self.in_flight = None;
        self.finished = self.finished.max(run.covers);
        match outcome {
            Ok(()) => self.failure = None,
            Err(reason) => {
                self.failure = Some(reason);
                self.stats.runs_failed = self.stats.runs_failed.saturating_add(1);
            }
        }
    }

    /// 在确证运行者不会再结算时，把在途派生判定为遗弃并以失败收尾。
    ///
    /// 仅限已经观测到运行者死亡的调用方使用（阻塞任务 panic、任务被取消、工作线程
    /// 消失）；闸门自己不读时钟，判断不了「跑得久」和「已经死了」的区别，所以绝不
    /// 能拿它当超时用。返回是否确实有在途派生被判定为遗弃。
    ///
    /// 判定之后在途位立即释放：被这次派生覆盖的票读到 `Failed`，排队位仍然保留，
    /// 下一次 `request` 会被要求去认领并重新跑完整派生。
    pub fn fail_abandoned_run(&mut self, reason: &'static str) -> bool {
        let Some(covers) = self.in_flight.take() else {
            return false;
        };
        self.finished = self.finished.max(covers);
        self.failure = Some(reason);
        self.stats.runs_failed = self.stats.runs_failed.saturating_add(1);
        self.stats.runs_abandoned = self.stats.runs_abandoned.saturating_add(1);
        true
    }

    /// 读取一张票的当前状态；调用方在被唤醒后读取一次即可答复。
    #[must_use]
    pub fn observe(&self, ticket: RefreshTicket) -> RefreshWait {
        if self.closed {
            return RefreshWait::Closed;
        }
        if ticket.0 > self.finished {
            return RefreshWait::Running;
        }
        self.failure
            .map_or(RefreshWait::Completed, RefreshWait::Failed)
    }

    /// 关闭闸门：丢弃排队位，之后的请求一律被拒，未完成的票读到 `Closed`。
    ///
    /// 已认领的在途派生仍须结算或判定为遗弃；这两者都不会让关闭后的票重新变成完成。
    pub fn close(&mut self) {
        self.closed = true;
        self.pending = false;
    }

    /// 闸门是否已关闭。
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.closed
    }

    /// 是否已有派生在途或排队；用于调用方判断要不要唤醒运行者。
    #[must_use]
    pub fn is_busy(&self) -> bool {
        self.in_flight.is_some() || self.pending
    }

    /// 可核对的合流计数。
    #[must_use]
    pub fn stats(&self) -> RefreshGateStats {
        self.stats
    }
}
