//! 并发刷新合流：任意数量的 `?refresh=true` 共享同一次派生，结算由派生自己完成。
//!
//! 只读副本侧过去用一把 `tokio::sync::Mutex` 串行化刷新：N 个并发请求各自跑一遍
//! 全量 [`load`]，只是排队跑，没有合流。现在改由 [`RefreshGate`] 受理——闸门保证任一
//! 时刻至多一次派生在途、至多一次派生排队，因此任意数量的并发请求最多换来两次派生，
//! 且排队的那次一定开始于这些请求之后，不会把更旧的结果当成更新的结果交付。
//!
//! 结算写在阻塞闭包里，不由 HTTP future 代持：客户端取消只丢弃等待，不会提前释放
//! 真实作业，也不会留下无人结算的在途派生。闭包连结算都做不到（panic 或阻塞任务
//! 被取消）时，只有 `spawn_blocking` 的 `JoinError` 能证明运行者已死，那里是
//! [`RefreshGate::fail_abandoned_run`] 唯一正当的调用点：闸门自己不读时钟，分不清
//! 「跑得久」和「已经死了」，绝不能拿它当超时用。收尸放在独立任务里而不是请求
//! future 里，是因为发起请求的那一方随时可能被取消，而在途派生必须有人收尾。

use std::{
    io::Write,
    sync::{Arc, Mutex, MutexGuard},
};

use axum::http::StatusCode;
use studiolo_vault_broker::ReadOnlyVault;
use studiolo_workbench::{RefreshClosed, RefreshGate, RefreshRun, RefreshTicket, RefreshWait};
use tokio::sync::Notify;

use crate::{ApiError, AppState, LocalError, Source, ViewStore, load};

/// 派生失败时答复给客户端的固定说明。底层错误可能带库内相对路径与英文诊断，
/// 只写服务端日志（标准错误），不进 HTTP 正文。
pub(crate) const REFRESH_FAILED: &str = "刷新派生失败，请查看服务端日志。";

/// 闸门只携带静态理由；对外答复与状态码留在这里，和失败位在同一把锁下更新。
struct Failure {
    status: StatusCode,
    message: String,
}

struct Shared {
    gate: RefreshGate,
    failure: Option<Failure>,
}

impl Shared {
    /// 把一张票读到的失败翻译成答复；详情缺失时退回闸门自己的静态理由，不编造原因。
    fn reply(&self, reason: &'static str) -> ApiError {
        self.failure.as_ref().map_or_else(
            || ApiError(StatusCode::SERVICE_UNAVAILABLE, reason.to_owned()),
            |failure| ApiError(failure.status, failure.message.clone()),
        )
    }
}

/// 本机只读副本的刷新合流闸门：状态机加唤醒手段，自身不做 I/O、不读时钟。
pub(crate) struct RefreshCoalescer {
    /// 闸门与失败详情共用一把短锁，保证「读到 `Failed`」与「读到详情」永远一致。
    state: Mutex<Shared>,
    settled: Notify,
}

impl Default for RefreshCoalescer {
    fn default() -> Self {
        Self::new()
    }
}

/// 一次受理之后该做的事；由闸门在同一把锁内一次性判定，调用方不再二次决策。
enum Progress {
    /// 本票已有结论，直接答复。
    Settled(Result<(), ApiError>),
    /// 本请求认领到了派生，必须把它跑完并结算。
    Claimed(RefreshRun),
    /// 已有别人在跑，等待被唤醒后重新观测。
    Wait,
}

impl RefreshCoalescer {
    pub(crate) fn new() -> Self {
        Self {
            state: Mutex::new(Shared {
                gate: RefreshGate::new(),
                failure: None,
            }),
            settled: Notify::new(),
        }
    }

    /// 临界区内只有纯状态转换，不会 panic；万一中毒也接着用，不把刷新永久锁死。
    fn shared(&self) -> MutexGuard<'_, Shared> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// 受理一次刷新请求；闸门关闭时如实拒绝，不假装刷新已经排上队。
    fn request(&self, reason: &'static str) -> Result<RefreshTicket, ApiError> {
        self.shared()
            .gate
            .request(reason)
            .map(|admission| admission.ticket())
            .map_err(|closed: RefreshClosed| {
                ApiError(StatusCode::SERVICE_UNAVAILABLE, closed.to_string())
            })
    }

    /// 读一次本票状态，顺带在仍未结算时尝试接手排队中的派生。
    fn progress(&self, ticket: RefreshTicket) -> Progress {
        let mut shared = self.shared();
        match shared.gate.observe(ticket) {
            RefreshWait::Completed => Progress::Settled(Ok(())),
            RefreshWait::Failed(reason) => Progress::Settled(Err(shared.reply(reason))),
            RefreshWait::Closed => Progress::Settled(Err(ApiError(
                StatusCode::SERVICE_UNAVAILABLE,
                RefreshClosed.to_string(),
            ))),
            RefreshWait::Running => shared
                .gate
                .claim()
                .map_or(Progress::Wait, Progress::Claimed),
        }
    }

    /// 结算一次派生并唤醒全部等待者；被闸门识别为迟到的结算连同理由一并忽略。
    fn settle(&self, run: RefreshRun, outcome: Result<(), LocalError>) {
        let mut shared = self.shared();
        let ignored = shared.gate.stats().settles_ignored;
        let failure = outcome.err().map(|error| Failure {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            message: public_message(&error),
        });
        let verdict = if failure.is_some() {
            Err("刷新派生失败")
        } else {
            Ok(())
        };
        shared.gate.settle(run, verdict);
        if shared.gate.stats().settles_ignored == ignored {
            // 只有被采纳的结算才改写详情，免得死而复生的运行者抹掉等待者已读到的结论。
            shared.failure = failure;
        }
        drop(shared);
        self.settled.notify_waiters();
    }

    /// 确证运行者已死之后释放在途位；没有在途派生时什么都不改。
    fn abandon(&self, reason: &'static str) {
        let mut shared = self.shared();
        if shared.gate.fail_abandoned_run(reason) {
            shared.failure = Some(Failure {
                status: StatusCode::SERVICE_UNAVAILABLE,
                message: reason.to_owned(),
            });
        }
        drop(shared);
        self.settled.notify_waiters();
    }

    /// 可核对的合流计数；供契约测试断言「N 个请求只换来几次派生」。
    #[cfg(test)]
    pub(crate) fn stats(&self) -> studiolo_workbench::RefreshGateStats {
        self.shared().gate.stats()
    }

    /// 是否仍有派生在途或排队；供契约测试断言取消不会释放真实作业。
    #[cfg(test)]
    pub(crate) fn is_busy(&self) -> bool {
        self.shared().gate.is_busy()
    }
}

/// 对外答复：固定句子；副本扫描不完整时只追加诊断条数，绝不带路径或底层英文原因。
fn public_message(error: &LocalError) -> String {
    match error {
        LocalError::Incomplete { total, .. } => {
            format!("{REFRESH_FAILED}副本扫描共有 {total} 条诊断。")
        }
        _ => REFRESH_FAILED.to_owned(),
    }
}

/// 完整原因只进服务端日志；诊断已在派生处收成有界摘录，这里再转义控制字符，
/// 免得库内文件名伪造日志行。写日志失败不改变答复。
fn log_failure(error: &LocalError) {
    let _ = writeln!(
        std::io::stderr(),
        "刷新派生失败：{}",
        error.to_string().escape_debug()
    );
}

pub(crate) async fn run(state: &AppState) -> Result<(), ApiError> {
    match &state.source {
        Source::Writer(workbench) => workbench
            .refresh()
            .await
            .map_err(|_| ApiError::unavailable()),
        Source::Replica(vault) => replica(state, vault).await,
    }
}

/// 只读副本侧的刷新：受理一张票，然后要么接手派生，要么等自己的票被覆盖。
async fn replica(state: &AppState, vault: &Arc<ReadOnlyVault>) -> Result<(), ApiError> {
    let ticket = state.refresh.request("http")?;
    #[cfg(test)]
    state.refresh_probe.admitted();
    loop {
        // 先登记唤醒再读状态：结算恰好发生在两者之间时也不会漏掉这次唤醒。
        let mut settled = std::pin::pin!(state.refresh.settled.notified());
        settled.as_mut().enable();
        match state.refresh.progress(ticket) {
            Progress::Settled(result) => return result,
            Progress::Claimed(run) => start(state, vault, run),
            Progress::Wait => settled.await,
        }
    }
}

/// 把认领到的派生交给阻塞线程，并另派一个收尸任务观测运行者是否死亡。
fn start(state: &AppState, vault: &Arc<ReadOnlyVault>, run: RefreshRun) {
    let runner = Arc::clone(&state.refresh);
    let vault = Arc::clone(vault);
    let view = state.current.clone();
    #[cfg(test)]
    let probe = Arc::clone(&state.refresh_probe);
    let running = tokio::task::spawn_blocking(move || {
        #[cfg(test)]
        probe.enter();
        let outcome = derive(&vault, &view);
        if let Err(error) = &outcome {
            // 完整原因在阻塞线程上写服务端日志；闸门自身仍不做 I/O，答复只给固定说明。
            log_failure(error);
        }
        // 派生自己结算：HTTP 请求被取消也不会留下无人结算的在途派生。
        runner.settle(run, outcome);
    });
    let owner = Arc::clone(&state.refresh);
    tokio::spawn(async move {
        if running.await.is_err() {
            // 运行者已死（panic 或阻塞任务被取消），派生句柄随它消失，永远不会结算。
            owner.abandon("刷新任务已死亡");
        }
    });
}

/// 重扫只读副本并整代发布；失败时保留上一代完整视图，不发布半成品。
fn derive(vault: &ReadOnlyVault, view: &ViewStore) -> Result<(), LocalError> {
    let next = view
        .snapshot()?
        .generation
        .number
        .checked_add(1)
        .ok_or(LocalError::Unavailable)?;
    let replacement = load(vault, next)?;
    let previous = view
        .0
        .write()
        .map_err(|_| LocalError::Unavailable)?
        .generation
        .replace(Arc::new(replacement));
    drop(previous);
    Ok(())
}
