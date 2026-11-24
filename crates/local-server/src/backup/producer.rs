//! 可信生产者：答队列命令、按固定策略安排总封存，把带键的审计请求交给原准入端口。
//!
//! 主服务进程里没有封存定时器：只有收到 `schedule` 才判断做不做，判断只看固定配置和账本，
//! 请求里没有任何可以影响结果的参数。序号由这里发，原意图在冻结之前落账。
//! 回收过期暂存和总封存都在后台线程里做，连接只拿到「做了／还有没做完的／还没到」。
//! 回收另外由监听循环按固定周期触发（`reclaim`）：只回收、不冻结，定时器停了也不会让过期副本一直留在盘上。
//! 健康页经 [`BackupJobsProbe`] 只读账本里的备份事件：不写账本、不补失败结果、不安排封存。
//! 队列的进度只在内存里，每次启动从账本第一条读起：监听起来后由后台线程把它读到末尾（`catch_up` 子模块），
//! 请求不用从头推。
//! 仓库抽查（`check` 子模块）：worker 空闲时问该不该查，生产者按账本定节奏和组号、先落原意图；
//! worker 跑完交结果，生产者核原意图再落账。
mod catch_up;
mod check;

use super::{
    admission::BackupAdmission,
    audit::BackupAuditPort,
    evidence::{BackupJobs, BackupJobsProbe},
    queue::Queue,
};
use crate::checkpoint::{
    CheckpointInputs, CheckpointTimes, OwnerExportPaths, SealFailure, StepTimes, trusted_catalog,
};
use std::{
    num::NonZeroU64,
    path::PathBuf,
    sync::{Arc, Condvar, Mutex, PoisonError},
    thread::JoinHandle,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use studiolo_audit_ledger::{AuditError, AuditFilter, AuditLedger, SqliteAuditLedger};
use studiolo_backup::{
    checkpoint::{
        manifest::ManifestInputs,
        staging::{StagingArea, StagingError, reclaim_staging},
    },
    worker::{
        audit::{
            BackupAuditRequest, BackupQueueRequest, QueueAnswer, QueueCommand,
            RepositoryCheckReport,
        },
        job::{backup_failure, backup_intent},
        recovery_point::BackupTargetKind,
        state::{FailureReason, JobKey},
    },
};
use studiolo_core::checkpoint::{
    CheckpointError, CheckpointId, FreezeBudget, FreezeRequest, InstantMillis,
};

/// 原生 worker 只接专用的仅追加目标；生产者和它用同一个固定类别。
pub const TARGET: BackupTargetKind = BackupTargetKind::DedicatedAppendOnlyTarget;
/// 一次读账本的页大小和时间预算；没读完就先回「暂不可用」，进度留着下次接着读。
const PAGE: usize = 512;
const SCAN_BUDGET: Duration = Duration::from_secs(5);
/// 回收过期暂存的时间预算。
const RECLAIM_BUDGET: Duration = Duration::from_secs(30);
/// 冻结窗口放开之后，剩下的回执核对、导出、整本验链、过滤视图和发布一共能用多久（BUG-045）。
/// 比最短封存间隔短；169 上推算 40～60 秒。另受封存正文保留期约束，不会做到副本过期以后。
/// 截止时间从封存开始算，冻结窗口也在它之内；看板巡检连续遇到冻结的封顶时长按它定（`board::MAX_FROZEN`）。
pub(crate) const SEAL_BUDGET: Duration = Duration::from_secs(300);

/// 可信启动层从私有备份配置（`--backup-config`）和启动选项算出来的固定事实；运行期间不变。
pub struct ProducerPlan {
    /// 独立 worker 的系统账号；只有它能连备份 socket。
    pub worker_uid: u32,
    /// 专用读取组。
    pub reader_gid: u32,
    /// 封存的固定父目录。
    pub staging_parent: PathBuf,
    /// 不进备份的临时数据根。
    pub ephemeral_root: PathBuf,
    /// 各 owner 正在服务的数据根；只用来核对它们和封存目录互不重叠。
    pub source_roots: Vec<PathBuf>,
    /// 两次尝试之间的最小间隔：从上一次准入或上一次尝试（含失败的）算起，取较晚的那个。
    pub min_interval_millis: u64,
    /// 主人批准的 RPO：健康页「恢复点年龄」超过它算降级。
    pub rpo_millis: u64,
    /// 封存正文从内容截止起保留多久。
    pub source_retention_millis: u64,
    /// 一次冻结窗口的预算。
    pub freeze_budget: FreezeBudget,
    /// 清单里登记的固定包、部署单元和秘密引用。
    pub manifest: ManifestInputs,
}

#[derive(Default)]
struct State {
    queue: Queue,
    /// 后台线程（回收，可能还有一次封存）正在跑。
    busy: bool,
}

/// 正在服务的原实例加固定配置；不能由请求构造。
pub struct BackupProducer {
    ledger: Arc<SqliteAuditLedger>,
    inputs: Arc<CheckpointInputs>,
    plan: ProducerPlan,
    state: Mutex<State>,
    background: Mutex<Option<JoinHandle<()>>>,
    /// 后台线程还可能进入或正在冻结窗口里：线程起来前置上，窗口放开或线程结束时复位（`settle_window` 等它）。
    window: Window,
    /// 用例用：封存放开冻结窗口之后停在这里，等用例放行（演练关停时封存正在放开之后那段）。
    #[cfg(test)]
    pub(crate) hold_after_release: Mutex<Option<std::sync::mpsc::Receiver<()>>>,
    /// 用例用：封存打出的分步日志行另收一份，核生产者把同一份分步接到了日志里（BUG-050）。
    #[cfg(test)]
    pub(crate) seal_lines: Mutex<Vec<String>>,
    /// 开始服务的时刻；账本里从没有成功时，逾期从它和第一次尝试里较早的那个起算。
    serving_since_ms: Option<i64>,
}

impl BackupProducer {
    /// 只接收启动层保留的原账本和原 owner 实例。
    pub fn new(
        ledger: Arc<SqliteAuditLedger>,
        inputs: Arc<CheckpointInputs>,
        plan: ProducerPlan,
    ) -> Arc<Self> {
        Arc::new(Self {
            ledger,
            inputs,
            plan,
            state: Mutex::new(State::default()),
            background: Mutex::new(None),
            window: Window::default(),
            #[cfg(test)]
            hold_after_release: Mutex::new(None),
            #[cfg(test)]
            seal_lines: Mutex::new(Vec::new()),
            serving_since_ms: now_millis().ok(),
        })
    }

    /// 批准的 worker 账号和专用读取组；监听层按它们核对 socket 和对端。
    pub fn identities(&self) -> (u32, u32) {
        (self.plan.worker_uid, self.plan.reader_gid)
    }

    /// 已接受的连接：实际 peer 必须是批准的独立 worker 账号，一问一答。
    #[cfg(target_os = "linux")]
    pub fn serve_connection(
        self: &Arc<Self>,
        stream: std::os::unix::net::UnixStream,
        deadline: Instant,
    ) -> Result<(), studiolo_backup::worker::audit::transport::AuditTransportError> {
        use studiolo_backup::worker::audit::transport::{AuditSocket, AuditTransportError};
        if self.plan.worker_uid == rustix::process::geteuid().as_raw() {
            return Err(AuditTransportError::Peer);
        }
        let mut socket = AuditSocket::new(stream, self.plan.worker_uid, deadline)?;
        let bytes = socket.receive()?;
        let response = self
            .dispatch(&bytes)
            .map_err(|_| AuditTransportError::Unavailable)?;
        socket.send(&response)
    }

    /// 单请求单答复。带键的请求每次都从账本重开原准入，不缓存授权。
    pub fn dispatch(self: &Arc<Self>, bytes: &[u8]) -> Result<Vec<u8>, AuditError> {
        if let Ok(request) = BackupQueueRequest::decode(bytes) {
            if request.target() != TARGET {
                return Err(AuditError::Serialization);
            }
            let answer = match request.command() {
                QueueCommand::NextPending => self.next_pending()?,
                QueueCommand::Schedule => self.schedule()?,
                QueueCommand::RepositoryCheck => self.repository_check()?,
            };
            return request
                .encode_reply(&answer)
                .map_err(|_| AuditError::CorruptStore);
        }
        if let Ok(request) = RepositoryCheckReport::decode(bytes) {
            if request.target() != TARGET {
                return Err(AuditError::Serialization);
            }
            return self.report_check(&request);
        }
        let request = BackupAuditRequest::decode(bytes).map_err(|_| AuditError::Serialization)?;
        if request.target() != TARGET {
            return Err(AuditError::Serialization);
        }
        let admission = BackupAdmission::resume(self.ledger.clone(), request.key(), TARGET)?;
        BackupAuditPort::new(admission).dispatch(bytes)
    }

    /// 等后台线程整个做完，含放开之后的导出和发布。关停不用它（只等窗口，见 `settle_window`）。
    pub fn settle(&self) {
        let handle = self.background.lock().ok().and_then(|mut slot| slot.take());
        if let Some(handle) = handle {
            let _ = handle.join();
        }
    }

    /// 关停用：等后台线程放开冻结窗口（或做完）就返回，不等放开之后的拷贝和发布（BUG-045）。
    /// 那一段随进程退出丢下，和「封存做到一半被杀」走同一条路：暂存由下次启动回收，
    /// 没写结果的原意图由下一次 `schedule` 补失败结果。
    pub fn settle_window(&self) {
        self.window.wait();
    }

    #[cfg(test)]
    fn hold_after_release(&self) {
        let gate = self
            .hold_after_release
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(gate) = gate {
            let _ = gate.recv();
        }
    }

    /// 封存成功路上的分步日志行（窗口那一行、放开之后那一行）：打进 stderr，用例里另收一份。
    fn seal_line(&self, line: String) {
        eprintln!("{line}");
        #[cfg(test)]
        self.seal_lines
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(line);
    }

    fn next_pending(&self) -> Result<QueueAnswer, AuditError> {
        let mut state = self.state.lock().map_err(|_| AuditError::Unavailable)?;
        self.refresh(&mut state.queue)?;
        Ok(state
            .queue
            .next_pending()
            .map_or(QueueAnswer::Idle, |record| {
                QueueAnswer::Pending(Box::new(record.clone()))
            }))
    }

    /// 从上次读到的位置接着读账本。自己刚追加的事件也走这里读回，保证按账本顺序喂。
    fn refresh(&self, queue: &mut Queue) -> Result<(), AuditError> {
        let started = Instant::now();
        loop {
            let page = self.ledger.query(
                &AuditFilter::default()
                    .after_offset(queue.scanned())
                    .with_limit(PAGE),
            )?;
            for record in &page {
                queue.absorb(TARGET, record);
            }
            if page.len() < PAGE {
                return Ok(());
            }
            if started.elapsed() >= SCAN_BUDGET {
                return Err(AuditError::Unavailable);
            }
        }
    }

    fn schedule(self: &Arc<Self>) -> Result<QueueAnswer, AuditError> {
        let mut state = self.state.lock().map_err(|_| AuditError::Unavailable)?;
        if state.busy {
            return Ok(QueueAnswer::Outstanding);
        }
        self.refresh(&mut state.queue)?;
        // 上一次封存没做成、失败结果又没写进去留下的原意图：先把失败结果补上。
        let orphans = state.queue.orphans();
        for key in &orphans {
            self.ledger.append(
                backup_failure(key, FailureReason::BarrierNotEstablished)
                    .map_err(|_| AuditError::Serialization)?,
            )?;
        }
        if !orphans.is_empty() {
            self.refresh(&mut state.queue)?;
        }
        let (answer, job) = self.decide(&mut state.queue)?;
        state.busy = true;
        drop(state);
        self.launch(job).map(|()| answer)
    }

    /// 只回收过期暂存：不安排封存、不冻结、不读账本。监听循环起来时做一次，之后按固定周期做；
    /// 后台线程在忙（回收或封存）就跳过这一轮。
    pub fn reclaim(self: &Arc<Self>) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        if state.busy {
            return;
        }
        state.busy = true;
        drop(state);
        // 线程没起来时 `launch` 已经复位 busy，下一轮再试。
        let _ = self.launch(None);
    }

    /// 起后台线程（回收，`job` 有值时接着做这一次封存）。调用前 busy 已经置上。
    fn launch(self: &Arc<Self>, job: Option<JobKey>) -> Result<(), AuditError> {
        let producer = Arc::clone(self);
        // 线程起来之前置上：关停这时来问，也要等这个线程走过窗口。
        self.window.open();
        let spawned = std::thread::Builder::new()
            .name("backup-producer".to_owned())
            .spawn(move || producer.housekeep(job));
        match spawned {
            Ok(handle) => {
                // 上一个线程已经复位过 busy（否则走不到这里），它的句柄直接换掉。
                if let Ok(mut slot) = self.background.lock() {
                    *slot = Some(handle);
                }
                Ok(())
            }
            Err(_) => {
                // 线程没起来：已写的原意图留在账本里，下一次 `schedule` 会给它补失败结果。
                self.window.close();
                if let Ok(mut state) = self.state.lock() {
                    state.busy = false;
                }
                Err(AuditError::Unavailable)
            }
        }
    }

    /// 只看账本和固定配置：到期且没有没做完的作业，就发下一个序号并把原意图写进账本。
    fn decide(&self, queue: &mut Queue) -> Result<(QueueAnswer, Option<JobKey>), AuditError> {
        if queue.outstanding() {
            return Ok((QueueAnswer::Outstanding, None));
        }
        let now = now_millis()?;
        let interval = i64::try_from(self.plan.min_interval_millis).unwrap_or(i64::MAX);
        if !due(
            now,
            interval,
            queue.last_admission_ms(),
            queue.last_attempt_ms(),
        ) {
            return Ok((QueueAnswer::NotDue, None));
        }
        let sequence = queue
            .next_sequence()
            .and_then(NonZeroU64::new)
            .ok_or(AuditError::Unavailable)?;
        let key = JobKey {
            sequence,
            checkpoint_id: CheckpointId::parse(format!("scheduled-{sequence}-{now}"))
                .map_err(|_| AuditError::Serialization)?,
        };
        self.ledger
            .append(backup_intent(&key, TARGET).map_err(|_| AuditError::Serialization)?)?;
        self.refresh(queue)?;
        Ok((QueueAnswer::Started, Some(key)))
    }

    /// 后台线程：先回收过期暂存，再做这一次封存（如果安排了）。
    fn housekeep(&self, job: Option<JobKey>) {
        // 正常返回和 panic 展开都要复位 busy，否则之后的 `schedule` 永远回 `outstanding`。
        let _idle = Idle(&self.state);
        // 同样：线程不管怎么结束，关停都不能一直等窗口。
        let _window = Closing(&self.window);
        // 回收不成不挡后面：登记损坏或别的进程占着时，新封存自己会被暂存层拒绝并记失败。
        if let Err(error) = reclaim_staging(
            &self.plan.staging_parent,
            Some(self.plan.reader_gid),
            Instant::now() + RECLAIM_BUDGET,
        ) {
            // 固定类别，不含路径和正文。
            eprintln!("备份暂存回收未完成：{error:?}");
        }
        if let Some(key) = job
            && let Err(reason) = self.seal(&key)
        {
            // 写不进去就留给下一次 `schedule` 补；这里不重试。
            if let Ok(event) = backup_failure(&key, reason) {
                let _ = self.ledger.append(event);
            }
            eprintln!("备份封存未完成：{reason:?}");
        }
    }

    /// 一次总封存：建暂存、冻结全部 owner 定格、放开屏障，再从定格导出并封存、写准入（BUG-045）。
    fn seal(&self, key: &JobKey) -> Result<(), FailureReason> {
        let budget = self.plan.freeze_budget;
        let retention_span = Duration::from_millis(self.plan.source_retention_millis);
        let deadline = Instant::now() + SEAL_BUDGET.min(retention_span);
        let area = StagingArea::create_for_reader(
            &self.plan.staging_parent,
            key.checkpoint_id.clone(),
            &self.plan.source_roots,
            &self.plan.ephemeral_root,
            self.plan.reader_gid,
            deadline,
        )
        .map_err(staging_reason)?;
        let sources = self
            .inputs
            .bind()
            .map_err(|error| stuck(error, &StepTimes::start()))?;
        // 冻结窗口自己的截止时间由冻结预算定（`CheckpointOwners::freeze_timed`），这里只量它多长，
        // 连同窗口里每一步各用了多久（BUG-050）。
        let window = Instant::now();
        let mut steps = StepTimes::start();
        let frozen = sources
            .freeze_timed(
                FreezeRequest {
                    checkpoint_id: key.checkpoint_id.clone(),
                    budget,
                },
                key,
                TARGET,
                OwnerExportPaths::under(area.export_root()),
                &mut steps,
            )
            .map_err(|error| stuck(error, &steps))?;
        // 全部 owner 冻结之后、放开之前取内容截止时刻：导出的就是这一刻，之后的改动不进这次封存。
        let cutoff = now_millis().map_err(barrier)?;
        let retention_millis = i64::try_from(self.plan.source_retention_millis).map_err(barrier)?;
        let expires = cutoff
            .checked_add(retention_millis)
            .ok_or(FailureReason::BarrierNotEstablished)?;
        let mut released = frozen
            .release_timed(deadline, &mut steps)
            .map_err(|error| stuck(error, &steps))?;
        self.window.close();
        self.seal_line(window_line(window.elapsed(), &steps));
        #[cfg(test)]
        self.hold_after_release();
        // 放开之后另起一份分步：导出、验链、封存各用了多久（BUG-050）。
        let opened = Instant::now();
        let mut after = StepTimes::start();
        let now = now_millis().map_err(after_release_step("取当前时间", &after))?;
        let sealed = released.seal_timed(
            area,
            trusted_catalog,
            self.plan.manifest.clone(),
            CheckpointTimes {
                content_cutoff: InstantMillis::from_millis(cutoff),
                expires_at: InstantMillis::from_millis(expires),
                now: InstantMillis::from_millis(now),
            },
            &mut after,
        );
        // 定格的连接和句柄先关掉再写准入。
        drop(released);
        let sealed = sealed.map_err(|failure| after_release(failure, &after))?;
        BackupAdmission::approve(self.ledger.clone(), key.clone(), TARGET, &sealed)
            .map_err(after_release_step("写准入", &after))?;
        after.mark("写准入");
        self.seal_line(released_done_line(opened.elapsed(), &after));
        Ok(())
    }
}

impl BackupJobsProbe for BackupProducer {
    /// 和 `schedule` 读同一份累计视图：从上次读到的位置接着读，读不完（超过扫描预算）就这一次不给结论。
    fn jobs(&self, now_ms: i64) -> Option<BackupJobs> {
        let mut state = self.state.lock().ok()?;
        self.refresh(&mut state.queue).ok()?;
        let queue = &state.queue;
        Some(BackupJobs {
            now_ms,
            serving_since_ms: self.serving_since_ms,
            first_attempt_ms: queue.first_attempt_ms(),
            last_success: queue.last_success(),
            last_failure: queue.last_failure(),
            latest_failed: queue.latest_failed(),
            outstanding: queue.oldest_open(),
            stuck_after_ms: self.plan.source_retention_millis,
            overdue_after_ms: self.plan.min_interval_millis.saturating_mul(2),
            recovery_point: queue.recovery_point(),
            recovery_point_budget_ms: self.plan.rpo_millis,
        })
    }

    fn repository_checks(&self, now_ms: i64) -> Option<super::RepositoryChecks> {
        self.repository_checks_at(now_ms)
    }
}

/// 后台线程结束时复位 busy 的守卫。锁被别处的 panic 毒化时也照样取回状态复位。
struct Idle<'a>(&'a Mutex<State>);

impl Drop for Idle<'_> {
    fn drop(&mut self) {
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        state.busy = false;
    }
}

/// 后台线程还可能进入或正在冻结窗口里（BUG-045）：关停只等这一段。
#[derive(Default)]
struct Window {
    open: Mutex<bool>,
    changed: Condvar,
}

impl Window {
    fn open(&self) {
        *self.open.lock().unwrap_or_else(PoisonError::into_inner) = true;
    }
    fn close(&self) {
        *self.open.lock().unwrap_or_else(PoisonError::into_inner) = false;
        self.changed.notify_all();
    }
    fn wait(&self) {
        let open = self.open.lock().unwrap_or_else(PoisonError::into_inner);
        drop(
            self.changed
                .wait_while(open, |open| *open)
                .unwrap_or_else(PoisonError::into_inner),
        );
    }
}

/// 后台线程结束时复位窗口标志的守卫（含 panic 展开）。
struct Closing<'a>(&'a Window);

impl Drop for Closing<'_> {
    fn drop(&mut self) {
        self.0.close();
    }
}

/// 到没到下一次尝试：距上一次准入、距上一次尝试（含失败的）都要满批准的最小间隔。
/// 失败不缩短间隔，确定性的失败也只会按间隔重试。时钟回退时差值为负，按「还没到」处理。
fn due(now: i64, interval: i64, last_admission: Option<i64>, last_attempt: Option<i64>) -> bool {
    [last_admission, last_attempt]
        .into_iter()
        .flatten()
        .all(|at| now.saturating_sub(at) >= interval)
}

/// 冻结、绑定、准入这些环节的失败都归成同一个固定原因，不带细节。
fn barrier<E>(_: E) -> FailureReason {
    FailureReason::BarrierNotEstablished
}

/// 绑定、冻结没做成：账本里照旧记同一个固定原因，日志里多说一句卡在哪个库、哪一类。
/// 只有这一句能分清是七个库里的哪一个；没有它，现场只能靠看文件访问顺序去猜（BUG-044）。
fn stuck(error: CheckpointError, steps: &StepTimes) -> FailureReason {
    eprintln!("{}", stuck_line(&error, steps));
    barrier(error)
}

/// 固定类别：哪个库、没排空还是超时还是不可用；错误类型自己的说法里没有路径和内容。
/// 带上窗口里已走过的步和用时（BUG-050）。
fn stuck_line(error: &CheckpointError, steps: &StepTimes) -> String {
    format!("备份封存卡在：{error}（已走：{}）", steps.render())
}

/// 放开之后没做成：账本里照旧记暂存层的固定原因，日志里多说一句是哪一步、哪个库（BUG-045）。
/// 最慢的几步挪到了放开之后，不在这里写，现场就又分不清卡在哪了（BUG-044 要消除的）。
fn after_release(failure: SealFailure, steps: &StepTimes) -> FailureReason {
    eprintln!("{}", released_line(&failure, steps));
    staging_reason(failure.staging())
}

/// 放开之后、`seal_timed` 以外的两步（取当前时间、写准入）没做成：同样写一行，只有固定的步骤名；
/// 账本里照旧是同一个固定原因。
fn after_release_step<'a, E>(
    step: &'static str,
    steps: &'a StepTimes,
) -> impl FnOnce(E) -> FailureReason + 'a {
    move |error| {
        eprintln!("{}", released_line(&step, steps));
        barrier(error)
    }
}

/// 固定类别：哪一步、哪个库或暂存层的哪一类；不含路径和内容。带上放开之后已走过的步（BUG-050）。
fn released_line(failure: &dyn std::fmt::Display, steps: &StepTimes) -> String {
    format!(
        "备份封存放开之后卡在：{failure}（已走：{}）",
        steps.render()
    )
}

/// 每次放开冻结窗口写一行：固定格式，毫秒数加窗口里每一步的毫秒数；真机看它判断窗口有没有随数据
/// 变长、是哪一步变长（BUG-045、BUG-050）。从补记登录库算起：补记要取完整操作许可，那时写请求也在等；
/// 到全部屏障放开为止。
fn window_line(window: Duration, steps: &StepTimes) -> String {
    format!(
        "备份封存冻结窗口：{} 毫秒（分步：{}）",
        window.as_millis(),
        steps.render()
    )
}

/// 放开之后到写完准入，整段做成时写一行：总毫秒数和每一步的毫秒数（BUG-050）。
fn released_done_line(total: Duration, steps: &StepTimes) -> String {
    format!(
        "备份封存放开之后：{} 毫秒（分步：{}）",
        total.as_millis(),
        steps.render()
    )
}

fn staging_reason(error: StagingError) -> FailureReason {
    match error {
        StagingError::Changed => FailureReason::SourceChangedBeforeSeal,
        StagingError::Limit => FailureReason::OutOfSpace,
        _ => FailureReason::BarrierNotEstablished,
    }
}

fn now_millis() -> Result<i64, AuditError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| i64::try_from(elapsed.as_millis()).ok())
        .ok_or(AuditError::Unavailable)
}

#[cfg(test)]
mod tests {
    use super::{
        Idle, SealFailure, State, StepTimes, due, released_done_line, released_line, stuck_line,
        window_line,
    };
    use std::{sync::Mutex, time::Duration};
    use studiolo_backup::checkpoint::staging::StagingError;
    use studiolo_core::checkpoint::{CheckpointError, CheckpointOwner};

    const HOUR: i64 = 3_600_000;

    #[test]
    fn a_failed_attempt_waits_the_whole_approved_interval() {
        let failed_at = 10 * HOUR;
        // 没有任何历史：可以做。
        assert!(due(failed_at, HOUR, None, None));
        // 失败之后一分钟、半小时都不重试，满一个间隔才重试。
        assert!(!due(failed_at + 61_000, HOUR, None, Some(failed_at)));
        assert!(!due(failed_at + HOUR / 2, HOUR, None, Some(failed_at)));
        assert!(due(failed_at + HOUR, HOUR, None, Some(failed_at)));
        // 准入和尝试取较晚的那个。
        assert!(!due(
            failed_at + HOUR,
            HOUR,
            Some(failed_at + HOUR / 2),
            Some(failed_at)
        ));
        // 时钟回退按「还没到」处理。
        assert!(!due(failed_at - 1, HOUR, None, Some(failed_at)));
    }

    #[test]
    fn the_log_line_for_a_seal_that_got_stuck_names_the_owner_and_the_kind() {
        // 七个库各有自己的名字；同一个库的五种失败说法不一样，现场一眼分得清。
        let none = StepTimes::start();
        for owner in CheckpointOwner::FREEZE_ORDER {
            let lines = [
                stuck_line(&CheckpointError::OwnerUnavailable { owner }, &none),
                stuck_line(&CheckpointError::NotDrained { owner }, &none),
                stuck_line(&CheckpointError::FreezeTimeout { owner }, &none),
                stuck_line(&CheckpointError::LimitExceeded { owner }, &none),
                stuck_line(&CheckpointError::SourceChanged { owner }, &none),
            ];
            for line in &lines {
                assert!(line.starts_with("备份封存卡在："), "{line}");
                assert!(line.contains(&format!("{owner:?}")), "{line}");
                assert!(!line.contains('/'), "{line}");
            }
            for (i, left) in lines.iter().enumerate() {
                assert!(lines[i + 1..].iter().all(|right| right != left), "{left}");
            }
        }
        let auth = stuck_line(
            &CheckpointError::OwnerUnavailable {
                owner: CheckpointOwner::Auth,
            },
            &none,
        );
        let audit = stuck_line(
            &CheckpointError::OwnerUnavailable {
                owner: CheckpointOwner::Audit,
            },
            &none,
        );
        assert_ne!(auth, audit);
    }

    #[test]
    fn the_log_line_after_release_names_the_step_and_the_owner_without_paths() {
        let owner = CheckpointOwner::Control;
        let broker = CheckpointOwner::Broker;
        let none = StepTimes::start();
        let lines = [
            released_line(
                &SealFailure::Verify(CheckpointError::OwnerUnavailable { owner }),
                &none,
            ),
            released_line(
                &SealFailure::Export(CheckpointError::OwnerUnavailable { owner }),
                &none,
            ),
            released_line(&SealFailure::Staging(StagingError::Changed), &none),
            released_line(
                &SealFailure::Receipts(CheckpointError::OwnerUnavailable { owner: broker }),
                &none,
            ),
            released_line(
                &SealFailure::Export(CheckpointError::SourceChanged { owner: broker }),
                &none,
            ),
            released_line(
                &SealFailure::Export(CheckpointError::LimitExceeded { owner: broker }),
                &none,
            ),
            released_line(&"取当前时间", &none),
            released_line(&"写准入", &none),
        ];
        for line in &lines {
            assert!(line.starts_with("备份封存放开之后卡在："), "{line}");
            assert!(!line.contains('/'), "{line}");
        }
        for (i, left) in lines.iter().enumerate() {
            assert!(lines[i + 1..].iter().all(|right| right != left), "{left}");
        }
        assert!(lines[0].contains("Control") && lines[1].contains("Control"));
        assert!(lines[2].contains("Changed"));
        assert!(lines[3..6].iter().all(|line| line.contains("Broker")));
        assert!(lines[3].contains("核写者回执"));
        // 账本里的固定原因照旧：核回执、导出没做成按暂存区不可用算，暂存层的照它自己的分类。
        for failure in [
            SealFailure::Verify(CheckpointError::OwnerUnavailable { owner }),
            SealFailure::Receipts(CheckpointError::OwnerUnavailable { owner: broker }),
            SealFailure::Export(CheckpointError::OwnerUnavailable { owner }),
        ] {
            assert_eq!(failure.staging(), StagingError::Unavailable);
        }
        assert_eq!(
            SealFailure::Staging(StagingError::Changed).staging(),
            StagingError::Changed
        );
    }

    /// 窗口那一行：原字面量「备份封存冻结窗口：N 毫秒」不变（发行装配按它核新程序），后面跟窗口里
    /// 每一步的毫秒数（BUG-050：169 上 26 秒只有一个总数，分不清是哪一步慢）。
    #[test]
    fn the_freeze_window_log_line_has_the_milliseconds_and_every_step() {
        let steps = StepTimes::of(&[("补记登录库", 3), ("定格写者两棵树", 1200)]);
        assert_eq!(
            window_line(Duration::from_micros(1_234_567), &steps),
            "备份封存冻结窗口：1234 毫秒（分步：补记登录库 3、定格写者两棵树 1200）"
        );
        assert_eq!(
            window_line(Duration::from_millis(7), &StepTimes::start()),
            "备份封存冻结窗口：7 毫秒（分步：无）"
        );
    }

    /// 放开之后整段做成：总毫秒数和每一步；没做成的那两种行带上已走过的步。
    #[test]
    fn the_lines_after_release_and_for_a_stuck_seal_carry_the_steps_taken() {
        let after = StepTimes::of(&[("校验与核回执", 10), ("写准入", 2)]);
        assert_eq!(
            released_done_line(Duration::from_millis(5_678), &after),
            "备份封存放开之后：5678 毫秒（分步：校验与核回执 10、写准入 2）"
        );
        assert_eq!(
            released_line(&"写准入", &after),
            "备份封存放开之后卡在：写准入（已走：校验与核回执 10、写准入 2）"
        );
        let error = CheckpointError::NotDrained {
            owner: CheckpointOwner::Audit,
        };
        let window = StepTimes::of(&[("补记登录库", 3), ("排空接入方", 0)]);
        assert_eq!(
            stuck_line(&error, &window),
            format!("备份封存卡在：{error}（已走：补记登录库 3、排空接入方 0）")
        );
    }

    #[test]
    fn a_panicking_background_thread_still_clears_busy() {
        let state = Mutex::new(State {
            busy: true,
            ..State::default()
        });
        let outcome = std::thread::scope(|scope| {
            scope
                .spawn(|| {
                    let _idle = Idle(&state);
                    panic!("封存中途 panic");
                })
                .join()
        });
        assert!(outcome.is_err());
        assert!(!state.lock().unwrap().busy);
    }
}
