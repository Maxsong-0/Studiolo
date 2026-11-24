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
    CheckpointInputs, CheckpointOwners, CheckpointTimes, HumanSide, OwnerExportPaths, SealFailure,
    StepTimes, trusted_catalog,
};
use std::{
    num::NonZeroU64,
    path::PathBuf,
    sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError},
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
use studiolo_core::checkpoint::{CheckpointId, FreezeBudget, FreezeRequest, InstantMillis};

/// 原生 worker 只接专用的仅追加目标；生产者和它用同一个固定类别。
pub const TARGET: BackupTargetKind = BackupTargetKind::DedicatedAppendOnlyTarget;
/// 一次读账本的页大小和时间预算；没读完就先回「暂不可用」，进度留着下次接着读。
const PAGE: usize = 512;
const SCAN_BUDGET: Duration = Duration::from_secs(5);
/// 回收过期暂存的时间预算。
const RECLAIM_BUDGET: Duration = Duration::from_secs(30);
/// 冻结窗口放开之后，剩下的回执核对、导出、整本验链、过滤视图和发布一共能用多久（BUG-045）。
/// 比最短封存间隔短；169 上推算 40～60 秒。另受封存正文保留期约束，不会做到副本过期以后。
/// 截止时间从等到人侧空闲之后算，等的时间不在里面，冻结窗口在它之内；看板巡检连续遇到冻结的封顶时长按它定
/// （`board::MAX_FROZEN`）。
pub(crate) const SEAL_BUDGET: Duration = Duration::from_secs(300);

/// 一次封存最多等人侧空下来多久（BUG-056）。在封存总预算和冻结预算之前，不占它们；等的时候网站、
/// 接入方都照常，等到上限这次封存照旧失败、按间隔再来。比最慢的一轮同步（169 上 71.6～72.8 秒，按 73 算）宽得多，
/// 加上封存总预算不超过一个批准的间隔（10 分钟）；前面的回收（[`RECLAIM_BUDGET`]）和后面 worker 的上传另算。
const IDLE_BUDGET: Duration = Duration::from_secs(300);
/// 等人侧空闲不到这么久就不打那一行：平时的日志不变。
const IDLE_NOTICE: Duration = Duration::from_secs(1);

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
    /// 两次尝试之间的最小间隔：从上一次尝试（原意图，含失败的）算起，怎么比见 [`due`]。
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
    /// 用例用：封存等到了人侧空闲、建好了暂存，开始冻结之前停在这里，等用例放行（演练这时来的关停）。
    #[cfg(test)]
    pub(crate) hold_before_freeze: Mutex<Option<std::sync::mpsc::Receiver<()>>>,
    /// 用例用：封存放开冻结窗口之后停在这里，等用例放行（演练关停时封存正在放开之后那段）。
    #[cfg(test)]
    pub(crate) hold_after_release: Mutex<Option<std::sync::mpsc::Receiver<()>>>,
    /// 用例用：经 `seal_line` 打的日志行另收一份，核生产者真的接到了日志里（BUG-050、BUG-056）。
    /// `housekeep` 的两行（暂存回收、封存未完成）不经它。
    #[cfg(test)]
    pub(crate) seal_lines: Mutex<Vec<String>>,
    /// 用例用：把等人侧空闲的上限改短；生产固定 [`IDLE_BUDGET`]。
    #[cfg(test)]
    pub(crate) idle_budget: Mutex<Duration>,
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
            hold_before_freeze: Mutex::new(None),
            #[cfg(test)]
            hold_after_release: Mutex::new(None),
            #[cfg(test)]
            seal_lines: Mutex::new(Vec::new()),
            #[cfg(test)]
            idle_budget: Mutex::new(IDLE_BUDGET),
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
    /// 还没开始冻结的封存（在等人侧空闲，或者刚等到）马上放弃，不用等它（BUG-056）；还在回收的，等回收走完
    /// （有上限，[`RECLAIM_BUDGET`]），不再往下封存。
    pub fn settle_window(&self) {
        self.window.wind_down();
    }

    /// 用例用：关停已经叫停了没有。
    #[cfg(test)]
    pub(crate) fn closing(&self) -> bool {
        self.window.closing_within(Duration::ZERO)
    }

    #[cfg(test)]
    fn hold(gate: &Mutex<Option<std::sync::mpsc::Receiver<()>>>) {
        let gate = gate.lock().unwrap_or_else(PoisonError::into_inner).take();
        if let Some(gate) = gate {
            let _ = gate.recv();
        }
    }

    /// 封存的日志行（等人侧空闲、窗口、放开之后、卡在哪一步）：打进 stderr，用例里另收一份。
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
        self.schedule_with(now_millis)
    }

    /// 同上，当前时刻由 `clock` 给，拿到状态锁、读完账本之后才取。生产只经上面那个入口用真实时钟；
    /// 测试用它把时间往后推，账本里的时刻照样是真实的落账时刻。
    pub(crate) fn schedule_with(
        self: &Arc<Self>,
        clock: impl FnOnce() -> Result<i64, AuditError>,
    ) -> Result<QueueAnswer, AuditError> {
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
        let (answer, job) = self.decide(&mut state.queue, clock)?;
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
    fn decide(
        &self,
        queue: &mut Queue,
        clock: impl FnOnce() -> Result<i64, AuditError>,
    ) -> Result<(QueueAnswer, Option<JobKey>), AuditError> {
        if queue.outstanding() {
            return Ok((QueueAnswer::Outstanding, None));
        }
        let now = clock()?;
        let interval = i64::try_from(self.plan.min_interval_millis).unwrap_or(i64::MAX);
        if !due(now, interval, queue.last_attempt_ms()) {
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

    /// 等人侧空下来（BUG-056）：不在预算里等、什么都不挡。等得久另打一行；关停叫停或等到上限就放弃这次封存。
    fn await_human_idle(&self, sources: &CheckpointOwners) -> Result<(), FailureReason> {
        let since = Instant::now();
        let until = since + self.idle_budget();
        match sources.await_human_idle(until, |span| self.window.closing_within(span)) {
            Ok(HumanSide::Idle) => {}
            Ok(HumanSide::Stopped) => return Err(self.unstarted(&CLOSING_DOWN)),
            Err(error) => return Err(self.unstarted(&format_args!("等人侧空闲：{error}"))),
        }
        let waited = since.elapsed();
        if waited >= IDLE_NOTICE {
            self.seal_line(idle_line(waited));
        }
        Ok(())
    }

    fn idle_budget(&self) -> Duration {
        #[cfg(not(test))]
        {
            IDLE_BUDGET
        }
        #[cfg(test)]
        {
            *self
                .idle_budget
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
        }
    }

    /// 一次总封存：等人侧空下来，建暂存、冻结全部 owner 定格、放开屏障，再从定格导出并封存、写准入（BUG-045）。
    fn seal(&self, key: &JobKey) -> Result<(), FailureReason> {
        let sources = self.inputs.bind().map_err(|error| self.unstarted(&error))?;
        // 先等再算预算：封存总预算、冻结预算和窗口那一行都不含等的时间（BUG-056）。
        self.await_human_idle(&sources)?;
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
        #[cfg(test)]
        Self::hold(&self.hold_before_freeze);
        // 从这里起算开始冻结：在这之前来的关停不再让它开始，之后来的等它放开（`settle_window`，BUG-056）。
        if self.window.closing_within(Duration::ZERO) {
            return Err(self.unstarted(&CLOSING_DOWN));
        }
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
            .map_err(|error| self.stuck(&error, &steps))?;
        // 全部 owner 冻结之后、放开之前取内容截止时刻：导出的就是这一刻，之后的改动不进这次封存。
        let cutoff = now_millis().map_err(barrier)?;
        let retention_millis = i64::try_from(self.plan.source_retention_millis).map_err(barrier)?;
        let expires = cutoff
            .checked_add(retention_millis)
            .ok_or(FailureReason::BarrierNotEstablished)?;
        let mut released = frozen
            .release_timed(deadline, &mut steps)
            .map_err(|error| self.stuck(&error, &steps))?;
        self.window.close();
        self.seal_line(window_line(window.elapsed(), &steps));
        #[cfg(test)]
        Self::hold(&self.hold_after_release);
        // 放开之后另起一份分步：导出、验链、封存各用了多久（BUG-050）。
        let opened = Instant::now();
        let mut after = StepTimes::start();
        let now = now_millis().map_err(self.after_release_step("取当前时间", &after))?;
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
        let sealed = sealed.map_err(|failure| self.after_release(failure, &after))?;
        BackupAdmission::approve(self.ledger.clone(), key.clone(), TARGET, &sealed)
            .map_err(self.after_release_step("写准入", &after))?;
        after.mark("写准入");
        self.seal_line(released_done_line(opened.elapsed(), &after));
        Ok(())
    }

    /// 绑定、冻结没做成，或者没开始就放弃：账本里照旧记同一个固定原因，日志里多说一句卡在哪个库、哪一类。
    /// 只有这一句能分清是七个库里的哪一个；没有它，现场只能靠看文件访问顺序去猜（BUG-044）。
    fn stuck(&self, why: &dyn std::fmt::Display, steps: &StepTimes) -> FailureReason {
        self.seal_line(stuck_line(why, steps));
        FailureReason::BarrierNotEstablished
    }

    /// 没开始冻结就放弃：步骤写「无」——什么都还没挡住。
    fn unstarted(&self, why: &dyn std::fmt::Display) -> FailureReason {
        self.stuck(why, &StepTimes::start())
    }

    /// 放开之后没做成：账本里照旧记暂存层的固定原因，日志里多说一句是哪一步、哪个库（BUG-045）。
    /// 最慢的几步挪到了放开之后，不在这里写，现场就又分不清卡在哪了（BUG-044 要消除的）。
    fn after_release(&self, failure: SealFailure, steps: &StepTimes) -> FailureReason {
        self.seal_line(released_line(&failure, steps));
        staging_reason(failure.staging())
    }

    /// 放开之后、`seal_timed` 以外的两步（取当前时间、写准入）没做成：同样写一行，只有固定的步骤名；
    /// 账本里照旧是同一个固定原因。
    fn after_release_step<'a, E>(
        &'a self,
        step: &'static str,
        steps: &'a StepTimes,
    ) -> impl FnOnce(E) -> FailureReason + 'a {
        move |error| {
            self.seal_line(released_line(&step, steps));
            barrier(error)
        }
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

/// 后台线程还可能进入或正在冻结窗口里（BUG-045）：关停等的就是这一段；还没开始冻结的封存关停时叫停，
/// 不等它等到人侧空闲（BUG-056）。
#[derive(Default)]
struct Window {
    state: Mutex<WindowState>,
    changed: Condvar,
}

#[derive(Default)]
struct WindowState {
    open: bool,
    /// 进程在关停：还没开始冻结的封存不再开始。置上就不复位。
    closing: bool,
}

impl Window {
    fn state(&self) -> MutexGuard<'_, WindowState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
    fn open(&self) {
        self.state().open = true;
    }
    fn close(&self) {
        self.state().open = false;
        self.changed.notify_all();
    }
    /// 关停：先叫停还没开始冻结的封存，再等窗口放开。
    fn wind_down(&self) {
        let mut state = self.state();
        state.closing = true;
        self.changed.notify_all();
        drop(
            self.changed
                .wait_while(state, |state| state.open)
                .unwrap_or_else(PoisonError::into_inner),
        );
    }
    /// 最多睡 `span`；已经在关停、或者睡着时开始关停，马上返回真。
    fn closing_within(&self, span: Duration) -> bool {
        self.changed
            .wait_timeout_while(self.state(), span, |state| !state.closing)
            .unwrap_or_else(PoisonError::into_inner)
            .0
            .closing
    }
}

/// 后台线程结束时复位窗口标志的守卫（含 panic 展开）。
struct Closing<'a>(&'a Window);

impl Drop for Closing<'_> {
    fn drop(&mut self) {
        self.0.close();
    }
}

/// 定时器的计划触发时刻都落在整分钟（`studiolo-backup-schedule.timer` 的 `OnCalendar`）；判断间隔之前，
/// 时刻先归到所在的整分钟。触发最多晚多久、命令最多跑多久，加起来不超过它（`deploy/backup-worker/test_units.py` 核）。
const TRIGGER_SLOT_MILLIS: i64 = 60_000;

/// 到没到下一次尝试：上一次尝试（含失败的）和现在各自的计划触发时刻之间，要满批准的最小间隔。
/// 失败不缩短间隔，确定性的失败也只会按间隔重试。时钟回退时差值为负，按「还没到」处理。
///
/// 从尝试（原意图落账）数，不从准入数：准入要等封存做完，从它数就会把下一次推到定时器的再下一格（BUG-057）；
/// 上一次还没做完由 `outstanding` 挡着，不会叠。按计划触发时刻比，是因为间隔正好是触发周期的整数倍：
/// 按实际毫秒比，两次触发晚到的先后差几毫秒就要多等一格。代价是单次最多比批准的间隔短不到一分钟
/// （只有不在整分钟触发时才会），但不累积：连着多少次，平均都不快于批准的间隔。
fn due(now: i64, interval: i64, last_attempt: Option<i64>) -> bool {
    last_attempt.is_none_or(|at| planned(now).saturating_sub(planned(at)) >= interval)
}

/// 这一刻所属的计划触发时刻：所在的整分钟。
fn planned(at: i64) -> i64 {
    at.saturating_sub(at.rem_euclid(TRIGGER_SLOT_MILLIS))
}

/// 冻结、绑定、准入这些环节的失败都归成同一个固定原因，不带细节。
fn barrier<E>(_: E) -> FailureReason {
    FailureReason::BarrierNotEstablished
}

/// 固定类别：哪个库、没排空还是超时还是不可用，或者在关停；错误类型自己的说法里没有路径和内容。
/// 带上窗口里已走过的步和用时（BUG-050）。
fn stuck_line(why: &dyn std::fmt::Display, steps: &StepTimes) -> String {
    format!("备份封存卡在：{why}（已走：{}）", steps.render())
}

/// 进程在关停，封存不再开始；账本里照旧是同一个固定原因。
const CLOSING_DOWN: &str = "正在关停，没开始冻结";

/// 等人侧空闲等了一会儿才等到：固定格式，只有毫秒数；真机看它知道封存晚了多久、是不是撞上了慢操作（BUG-056）。
fn idle_line(waited: Duration) -> String {
    format!("备份封存先等人侧空闲：{} 毫秒", waited.as_millis())
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
        IDLE_BUDGET, Idle, SEAL_BUDGET, SealFailure, State, StepTimes, due, idle_line,
        released_done_line, released_line, stuck_line, window_line,
    };
    use std::{sync::Mutex, time::Duration};
    use studiolo_backup::checkpoint::staging::StagingError;
    use studiolo_core::checkpoint::{CheckpointError, CheckpointOwner};

    const HOUR: i64 = 3_600_000;

    #[test]
    fn a_failed_attempt_waits_the_whole_approved_interval() {
        let failed_at = 10 * HOUR;
        // 没有任何历史：可以做。
        assert!(due(failed_at, HOUR, None));
        // 失败之后一分钟、半小时都不重试，满一个间隔才重试。
        assert!(!due(failed_at + 61_000, HOUR, Some(failed_at)));
        assert!(!due(failed_at + HOUR / 2, HOUR, Some(failed_at)));
        assert!(!due(failed_at + HOUR - 1, HOUR, Some(failed_at)));
        assert!(due(failed_at + HOUR, HOUR, Some(failed_at)));
        // 时钟回退按「还没到」处理。
        assert!(!due(failed_at - 1, HOUR, Some(failed_at)));
        assert!(!due(failed_at - HOUR, HOUR, Some(failed_at)));
    }

    #[test]
    fn the_interval_is_compared_between_planned_trigger_moments() {
        let planned = 10 * HOUR;
        // 这一分钟里不管晚到多久落的账，下一次都在整一个间隔之后的那个整分钟；前一毫秒还不到。
        for late in [0, 1, 8, 30_000, MINUTE - 1] {
            let attempted = planned + late;
            assert!(!due(planned + APPROVED - 1, APPROVED, Some(attempted)));
            assert!(due(planned + APPROVED, APPROVED, Some(attempted)));
        }
        // 晚过一分钟就算下一分钟的：退化成多等，不会提前。
        assert!(!due(planned + APPROVED, APPROVED, Some(planned + MINUTE)));
        // 间隔不是整分钟时往上取，不往下取。
        assert!(!due(planned + MINUTE, MINUTE + 1, Some(planned)));
        assert!(due(planned + 2 * MINUTE, MINUTE + 1, Some(planned)));
        // 比一分钟短的间隔也至少等到下一个整分钟。
        assert!(!due(planned + MINUTE - 1, 1, Some(planned)));
        assert!(due(planned + MINUTE, 1, Some(planned)));
    }

    #[test]
    fn asking_at_every_moment_never_makes_seals_more_frequent_than_approved() {
        // 触发方（worker 账号）一秒问一次、从一分钟的最后一刻开始：单次最多短不到一分钟，但不累积。
        let first = 10 * HOUR + MINUTE - 1;
        let mut attempts = vec![first];
        for now in (first..first + 24 * HOUR).step_by(1_000) {
            let last = attempts[attempts.len() - 1];
            if due(now, APPROVED, Some(last)) {
                assert!(now - last > APPROVED - MINUTE, "{last} → {now}");
                attempts.push(now);
            }
        }
        let gaps = i64::try_from(attempts.len()).unwrap() - 1;
        assert!(gaps > 100, "{gaps}");
        assert!(attempts[attempts.len() - 1] - first > gaps * APPROVED - MINUTE);
    }

    const MINUTE: i64 = 60_000;
    /// 169 上批准的间隔和定时器的周期（`deploy/backup-worker` 的用例核它们和部署文件一致）。
    const APPROVED: i64 = 10 * MINUTE;
    const TICK: i64 = 5 * MINUTE;

    /// 照 169 的样子走一天：定时器每 5 分钟问一次，每次晚到的毫秒数照 `late` 轮着取；封存成功，worker 在原意图之后
    /// 3.5 分钟收口，收口之前问到的一律不做。回每次开始的计划触发时刻。
    fn a_day_of_triggers(late: &[i64]) -> Vec<i64> {
        const CLOSED_AFTER: i64 = 210_000;
        // 判断之后要等原意图落账：记下的时刻比判断那一刻晚一点。
        const APPEND: i64 = 8;
        let mut started = Vec::new();
        let mut last: Option<i64> = None;
        for (n, late) in (0..24 * HOUR / TICK).zip(late.iter().cycle()) {
            let planned = n * TICK;
            let now = planned + late;
            if last.is_some_and(|at| now < at + CLOSED_AFTER) {
                continue;
            }
            if due(now, APPROVED, last) {
                started.push(planned);
                last = Some(now + APPEND);
            }
        }
        started
    }

    #[test]
    fn every_successful_seal_is_followed_by_the_next_one_exactly_one_approved_interval_later() {
        // 晚到：一点不晚、越来越早、越来越晚、忽早忽晚、到定时器精度上限。
        for late in [
            &[0][..],
            &[900, 600, 300, 0],
            &[0, 300, 600, 900],
            &[40, 3, 700, 12, 2_500, 1],
            &[30_000, 0, 29_999, 5],
        ] {
            let started = a_day_of_triggers(late);
            assert!(
                started.windows(2).all(|pair| pair[1] - pair[0] == APPROVED),
                "{late:?}: {started:?}"
            );
            assert_eq!(started.len(), 24 * 6, "{late:?}");
        }
    }

    #[test]
    fn a_trigger_that_is_missed_costs_one_tick_and_the_grid_recovers() {
        // 第 10 分钟那次没问成（连不上、上一次还没收口）：下一格补上，再下一次又是一个间隔之后。
        let last = 10 * HOUR + 20;
        let retried = 10 * HOUR + APPROVED + TICK + 3;
        assert!(due(retried, APPROVED, Some(last)));
        assert!(!due(retried + TICK, APPROVED, Some(retried)));
        assert!(due(
            10 * HOUR + 2 * APPROVED + TICK,
            APPROVED,
            Some(retried)
        ));
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

    /// 等人侧空闲和封存总预算两项加起来不超过 169 批准的间隔（BUG-056；批准的数和部署一致由 `test_units.py` 核）。
    /// 只是这两项的上限：前面的回收、后面 worker 的上传另算，真拖过了，下一格由 `outstanding` 挡着、再下一格补上。
    /// 反过来，等的上限要比 169 上见过的最慢一轮同步（71.6～72.8 秒，按 73 算）宽一倍以上，不然等了也白等。
    #[test]
    fn waiting_for_the_human_side_and_sealing_fit_in_the_approved_interval() {
        let approved = Duration::from_millis(APPROVED.try_into().unwrap());
        assert!(IDLE_BUDGET + SEAL_BUDGET <= approved);
        assert!(IDLE_BUDGET >= 2 * Duration::from_secs(73));
    }

    /// 等到了人侧空闲的那一行只有毫秒数；没等到、关停叫停的两行由 `human_idle.rs` 在真的封存里核（BUG-056）。
    #[test]
    fn the_line_about_waiting_for_the_human_side_is_fixed() {
        assert_eq!(
            idle_line(Duration::from_micros(61_234_567)),
            "备份封存先等人侧空闲：61234 毫秒"
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
