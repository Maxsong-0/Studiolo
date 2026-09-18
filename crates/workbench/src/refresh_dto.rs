//! 刷新闸门的公开值类型；只描述合流状态，不持有线程、锁、时钟或 I/O 能力。

/// 一次刷新请求的票号；单调递增，不复用，不跨闸门实例比较。
///
/// 票号只表示「覆盖到此请求为止的一次派生」，不表示代次编号，也不表示新鲜度水位。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct RefreshTicket(pub(crate) u64);

impl RefreshTicket {
    /// 票号的原始数值，仅用于日志与断言，不得据此推断代次或提交水位。
    #[must_use]
    pub fn value(self) -> u64 {
        self.0
    }
}

/// 请求被受理后调用方要承担的角色；两种角色都持有同一张可观测的票。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefreshAdmission {
    /// 本请求要负责去推进派生：必须循环 `claim` 与 `settle`，直到 `claim` 返回 `None`。
    ///
    /// 这是一份「去认领」的义务，不是「已经是运行者」的断言。并发请求可能同时
    /// 拿到 `Start`，但只有一个能认领到派生；`claim` 立刻返回 `None` 表示别人
    /// 已经接手，本请求转为等待自己的票即可，不得自己另跑一遍。
    Start(RefreshTicket),
    /// 已有派生在途并覆盖本请求；只等待观测结果，不得自己派生。
    Join(RefreshTicket),
}

impl RefreshAdmission {
    /// 本次受理拿到的票号，与角色无关。
    #[must_use]
    pub fn ticket(self) -> RefreshTicket {
        match self {
            Self::Start(ticket) | Self::Join(ticket) => ticket,
        }
    }

    /// 本次受理是否要负责去认领并推进派生。
    #[must_use]
    pub fn starts_run(self) -> bool {
        matches!(self, Self::Start(_))
    }
}

/// 一次已认领、尚未结算的派生。不可复制，只能由 `settle` 消费一次。
///
/// 持有者中止时也必须以失败结算；否则被覆盖的等待者不会被唤醒。持有者连结算都
/// 做不到（线程 panic、任务被取消）时，观测到这一事实的调用方改调
/// `RefreshGate::fail_abandoned_run` 收尾；此后这张票的迟到结算会被忽略。
#[derive(Debug)]
pub struct RefreshRun {
    pub(crate) covers: u64,
    pub(crate) reason: &'static str,
}

impl RefreshRun {
    /// 本次派生完成后可以交付的最高票号。
    #[must_use]
    pub fn covers(&self) -> RefreshTicket {
        RefreshTicket(self.covers)
    }

    /// 触发本次派生的第一个请求给出的理由，供调用方标记陈旧原因。
    ///
    /// 合流进同一次派生的后续请求的理由不会覆盖它，也不会被丢弃统计之外地保留。
    #[must_use]
    pub fn reason(&self) -> &'static str {
        self.reason
    }
}

/// 对某张票的当前观测结果；是状态读数，不是历史判决。
///
/// 一张被失败派生覆盖的票，在更晚的成功派生把失败清除后会读到 `Completed`；
/// 调用方应在被唤醒时读取一次并据此答复，不应把旧读数当成永久结论。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefreshWait {
    /// 覆盖本票的派生尚未结算，调用方继续等待。
    Running,
    /// 覆盖本票的派生已成功结算。
    Completed,
    /// 最近一次结算是失败，携带调用方自己的静态理由。
    Failed(&'static str),
    /// 闸门已关闭，本票不会再被任何派生覆盖。
    Closed,
}

/// 闸门已关闭，或票号已达上限而被动关闭；调用方不得再请求刷新。
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("刷新闸门已关闭，不再受理刷新请求。")]
pub struct RefreshClosed;

/// 可核对的合流证据；全部为单调计数，用于断言与运维观测。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RefreshGateStats {
    /// 受理过的刷新请求总数，含被合流的请求。
    pub admitted: u64,
    /// 实际开始过的派生次数；与请求数之差即被合流掉的请求数。
    pub runs_started: u64,
    /// 以失败收尾的派生次数，含被判定为遗弃的派生。
    pub runs_failed: u64,
    /// 因运行者死亡而被判定为遗弃的派生次数；它同时计入 `runs_failed`。
    pub runs_abandoned: u64,
    /// 被识别为陈旧而整体忽略的结算次数；正常协议下恒为零。
    pub settles_ignored: u64,
}

impl RefreshGateStats {
    /// 被合流掉、没有换来一次独立派生的请求数。
    #[must_use]
    pub fn coalesced(self) -> u64 {
        self.admitted.saturating_sub(self.runs_started)
    }
}
