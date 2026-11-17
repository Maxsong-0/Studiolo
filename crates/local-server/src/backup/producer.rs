//! 可信生产者：答队列命令、按固定策略安排总封存，把带键的审计请求交给原准入端口。
//!
//! 主服务进程里没有封存定时器：只有收到 `schedule` 才判断做不做，判断只看固定配置和账本，
//! 请求里没有任何可以影响结果的参数。序号由这里发，原意图在冻结之前落账。
//! 回收过期暂存和总封存都在后台线程里做，连接只拿到「做了／还有没做完的／还没到」。
//! 回收另外由监听循环按固定周期触发（`reclaim`）：只回收、不冻结，定时器停了也不会让过期副本一直留在盘上。
//! 健康页经 [`BackupJobsProbe`] 只读账本里的备份事件：不写账本、不补失败结果、不安排封存。
//! 仓库抽查（`check` 子模块）：worker 空闲时问该不该查，生产者按账本定节奏和组号、先落原意图；
//! worker 跑完交结果，生产者核原意图再落账。
mod check;

use super::{
    admission::BackupAdmission,
    audit::BackupAuditPort,
    evidence::{BackupJobs, BackupJobsProbe},
    queue::Queue,
};
use crate::checkpoint::{CheckpointInputs, CheckpointTimes, OwnerExportPaths, trusted_catalog};
use std::{
    num::NonZeroU64,
    path::PathBuf,
    sync::{Arc, Mutex, PoisonError},
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

/// 可信启动层从私有配置算出来的固定事实；运行期间不变。
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

    /// 等后台线程做完；关停时先停掉监听，再调用它，最后才停写者。
    pub fn settle(&self) {
        let handle = self.background.lock().ok().and_then(|mut slot| slot.take());
        if let Some(handle) = handle {
            let _ = handle.join();
        }
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

    /// 一次总封存：建暂存、冻结全部 owner、导出并封存、放开屏障、写准入。
    fn seal(&self, key: &JobKey) -> Result<(), FailureReason> {
        let budget = self.plan.freeze_budget;
        let deadline = Instant::now() + Duration::from_millis(u64::from(budget.millis()));
        let area = StagingArea::create_for_reader(
            &self.plan.staging_parent,
            key.checkpoint_id.clone(),
            &self.plan.source_roots,
            &self.plan.ephemeral_root,
            self.plan.reader_gid,
            deadline,
        )
        .map_err(staging_reason)?;
        let sources = self.inputs.bind().map_err(barrier)?;
        let frozen = sources
            .freeze(
                FreezeRequest {
                    checkpoint_id: key.checkpoint_id.clone(),
                    budget,
                },
                key,
                TARGET,
                OwnerExportPaths::under(area.export_root()),
            )
            .map_err(barrier)?;
        // 全部 owner 冻结之后才取内容截止时刻：此后到封存结束，内容不会再变。
        let cutoff = now_millis().map_err(barrier)?;
        let retention = i64::try_from(self.plan.source_retention_millis).map_err(barrier)?;
        let expires = cutoff
            .checked_add(retention)
            .ok_or(FailureReason::BarrierNotEstablished)?;
        let sealed = frozen.seal(
            area,
            trusted_catalog,
            self.plan.manifest.clone(),
            CheckpointTimes {
                content_cutoff: InstantMillis::from_millis(cutoff),
                expires_at: InstantMillis::from_millis(expires),
                now: InstantMillis::from_millis(cutoff),
            },
        );
        // 先放开全部屏障再写准入：准入要在冻结窗口之外追加进人侧账本。
        drop(frozen);
        let sealed = sealed.map_err(staging_reason)?;
        BackupAdmission::approve(self.ledger.clone(), key.clone(), TARGET, &sealed)
            .map(|_| ())
            .map_err(barrier)
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
    use super::{Idle, State, due};
    use std::sync::Mutex;

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
