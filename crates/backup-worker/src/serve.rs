//! 进程入口用的三件事：常驻循环、请生产者安排一次封存、显式初始化状态目录。
//!
//! 全部用固定路径和固定身份，没有任何参数。常驻循环只做一件事：问生产者「下一个待办」，
//! 对它走状态机的一步（决策表在 `BackupWorker::advance`）。不装信号处理：状态机本来就按
//! 崩溃恢复设计，被终止和崩溃走同一条恢复路径。
use crate::{
    AppendRunner, ApprovedJob, FinalManifestProbe, ProcessError, ResticCredentials, WorkerConfig,
    WorkerQueueClient,
};
use std::{
    cell::Cell,
    path::Path,
    time::{Duration, Instant},
};
use studiolo_audit_ledger::{AuditError, AuditEvent, AuditFilter, AuditLedger, AuditRecord};
use studiolo_backup::{
    checkpoint::staging::SealedCheckpoint,
    store::storage::BackupStore,
    worker::{
        audit::QueueAnswer,
        final_manifest::{
            FinalManifestDraft, FinalManifestLookup, FinalManifestReader, FinalManifestWriter,
        },
        job::{
            Advance, BackupWorker, JobOutcome, JobPorts, PortUnavailable, RemoteProbe, Transfer,
        },
        recovery_point::BackupTargetKind,
        restic::{CredentialHolder, ResticError, ResticRunner},
    },
};
use studiolo_process_safety::Residency;

/// worker 自己的作业状态目录；要事先建好，属主是 worker 账号、权限 0700。
const STATE_ROOT: &str = "/var/lib/studiolo-backup-worker/state";
/// 读配置、问队列这类短交互的窗口。
const SHORT: Duration = Duration::from_secs(10);
/// 一个作业一步的最长窗口；准入层自己还会压到两小时以内。
const JOB: Duration = Duration::from_secs(7200);
/// 没有待办、生产者不可达、或这一步没收口时，过多久再问。
const WAITING: Duration = Duration::from_secs(30);
/// 一个作业收口之后，过多久看下一个。
const SETTLED: Duration = Duration::from_secs(1);

/// 进程入口看到的固定失败类别；不带路径、键和子进程输出。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServeError {
    /// 秘密驻留面没能收紧；此时还没读任何凭据。
    Residency,
    /// 固定配置文件不可用，或当前账号不是配置里的 worker。
    Configuration,
    /// 状态目录不可用、被别的进程占着，或初始化时已经有状态。
    State,
    /// 生产者不可达或答复不合规。
    Producer,
    /// 子进程清理没能证实；必须退出，由服务管理器按重启策略处理。
    Shutdown,
}
impl std::fmt::Display for ServeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "备份 worker 未完成：{self:?}")
    }
}
impl std::error::Error for ServeError {}

/// `init`：显式初始化固定状态目录。已经有锁或状态就拒绝，不重置序号。
pub fn initialize() -> Result<(), ServeError> {
    BackupStore::open(Path::new(STATE_ROOT), true)
        .map(drop)
        .map_err(|_| ServeError::State)
}

/// `schedule`：请生产者判断该不该做一次封存。做不做由它按批准的间隔定，这里没有参数可传。
pub fn schedule() -> Result<QueueAnswer, ServeError> {
    let deadline = Instant::now() + SHORT;
    let config = WorkerConfig::load(deadline).map_err(|_| ServeError::Configuration)?;
    WorkerQueueClient::new(&config, deadline)
        .and_then(|queue| queue.schedule())
        .map_err(|_| ServeError::Producer)
}

/// `run`：常驻循环。只有必须退出时才返回，返回值就是退出原因。
pub fn run() -> ServeError {
    // 加固必须早于任何凭据读取；本平台声明支持却没生效就退出，不降级。
    let Ok(residency) = studiolo_process_safety::harden() else {
        return ServeError::Residency;
    };
    eprintln!("秘密驻留面：{residency}");
    let Ok(config) = WorkerConfig::load(Instant::now() + SHORT) else {
        return ServeError::Configuration;
    };
    let Ok(store) = BackupStore::open(Path::new(STATE_ROOT), false) else {
        return ServeError::State;
    };
    // 上一次进程在执行中途没了：只把那些阶段标成未知，不自动重做任何副作用。
    let recovered = BackupWorker::new(
        &Detached,
        &store,
        config.repository().clone(),
        CredentialHolder::BackupWorker,
        BackupTargetKind::DedicatedAppendOnlyTarget,
    )
    .and_then(|worker| worker.recover_interrupted());
    if recovered.is_err() {
        return ServeError::State;
    }
    loop {
        let pause = match round(&residency, &config, &store) {
            Ok(Round::Settled) => SETTLED,
            Ok(Round::Waiting) => WAITING,
            Err(stop) => return stop,
        };
        std::thread::sleep(pause);
    }
}

enum Round {
    /// 一个作业收口了，可以马上看下一个。
    Settled,
    /// 没有待办，或者这一步还没收口。
    Waiting,
}

fn round(
    residency: &Residency,
    config: &WorkerConfig,
    store: &BackupStore,
) -> Result<Round, ServeError> {
    let pending = WorkerQueueClient::new(config, Instant::now() + SHORT)
        .and_then(|queue| queue.next_pending());
    let key = match pending {
        Ok(Some(key)) => key,
        Ok(None) => return Ok(Round::Waiting),
        Err(_) => return Ok(waiting("生产者暂不可达")),
    };
    // 队列给的原键只是线索：这里用带键的准入请求重新取得批准，自报的键不能代替它。
    let Ok(job) = ApprovedJob::connect(config, key, Instant::now() + JOB) else {
        return Ok(waiting("原准入没有核对通过"));
    };
    let Ok(worker) = BackupWorker::new(
        job.audit(),
        store,
        config.repository().clone(),
        CredentialHolder::BackupWorker,
        job.target(),
    ) else {
        return Ok(waiting("状态机没有装配起来"));
    };
    let ports = Ports {
        job: &job,
        residency,
        poisoned: Cell::new(false),
    };
    let step = worker.advance(job.key(), &ports);
    if ports.poisoned.get() {
        return Err(ServeError::Shutdown);
    }
    Ok(match step {
        Ok(Advance::Outcome(
            outcome @ (JobOutcome::Succeeded | JobOutcome::Failed(_) | JobOutcome::Unverified(_)),
        )) => {
            eprintln!("备份作业已收口：{outcome:?}");
            Round::Settled
        }
        Ok(Advance::Outcome(outcome)) => {
            eprintln!("备份作业这一步没有收口：{outcome:?}");
            Round::Waiting
        }
        Ok(Advance::Deferred) => waiting("数据面这一轮没有装配起来"),
        Err(error) => {
            eprintln!("备份作业这一步被拒绝：{error}");
            Round::Waiting
        }
    })
}

/// 固定句子，不带路径、键和底层错误。
fn waiting(reason: &str) -> Round {
    eprintln!("备份 worker 等下一轮：{reason}");
    Round::Waiting
}

/// 启动恢复只改本地状态，不需要账本；任何账本调用都拒绝，免得被当成「已落账」。
struct Detached;
impl AuditLedger for Detached {
    fn append(&self, _: AuditEvent) -> Result<AuditRecord, AuditError> {
        Err(AuditError::Unavailable)
    }
    fn query(&self, _: &AuditFilter) -> Result<Vec<AuditRecord>, AuditError> {
        Err(AuditError::Unavailable)
    }
}

/// 一个已批准作业这一轮的外部能力。凭据每次装配时从固定系统凭据目录重新读。
struct Ports<'a, 'c> {
    job: &'a ApprovedJob<'c>,
    residency: &'a Residency,
    poisoned: Cell<bool>,
}
impl Ports<'_, '_> {
    fn credentials(&self) -> Result<ResticCredentials, PortUnavailable> {
        ResticCredentials::load(self.residency, self.job.config(), self.job.deadline())
            .map_err(|_| PortUnavailable)
    }
    /// 清理没能证实的错误要让整个进程退出，不能当成「这一轮没装配起来」。
    fn refuse<T>(&self, result: Result<T, ProcessError>) -> Result<T, PortUnavailable> {
        result.map_err(|error| {
            if error == ProcessError::CleanupFailed {
                self.poisoned.set(true);
            }
            PortUnavailable
        })
    }
}
impl JobPorts for Ports<'_, '_> {
    fn source_live(&self) -> bool {
        self.job.source_live()
    }
    fn transfer(&self) -> Result<Box<dyn Transfer + '_>, PortUnavailable> {
        let runner = self.refuse(AppendRunner::new(self.job, self.credentials()?))?;
        Ok(Box::new(Opened {
            runner,
            poisoned: &self.poisoned,
        }))
    }
    fn final_reader(&self) -> Result<Box<dyn FinalManifestReader + '_>, PortUnavailable> {
        let probe = self.refuse(FinalManifestProbe::new(self.job, self.credentials()?))?;
        Ok(Box::new(Reader {
            probe,
            poisoned: &self.poisoned,
        }))
    }
}

/// 原封存加追加执行器；用完时把「必须退出」的标记带回循环。
struct Opened<'a> {
    runner: AppendRunner,
    poisoned: &'a Cell<bool>,
}
impl Transfer for Opened<'_> {
    fn sealed(&self) -> &SealedCheckpoint {
        self.runner.sealed()
    }
    fn runner(&self) -> &dyn ResticRunner {
        &self.runner
    }
    fn probe(&self) -> &dyn RemoteProbe {
        &self.runner
    }
    fn finalizer(&self) -> &dyn FinalManifestWriter {
        &self.runner
    }
}
impl Drop for Opened<'_> {
    fn drop(&mut self) {
        if self.runner.requires_shutdown() {
            self.poisoned.set(true);
        }
    }
}

/// 最终清单的只读核对；不开正文。
struct Reader<'a> {
    probe: FinalManifestProbe,
    poisoned: &'a Cell<bool>,
}
impl FinalManifestReader for Reader<'_> {
    fn read_final(&self, draft: &FinalManifestDraft) -> Result<FinalManifestLookup, ResticError> {
        self.probe.read_final(draft)
    }
}
impl Drop for Reader<'_> {
    fn drop(&mut self) {
        if self.probe.requires_shutdown() {
            self.poisoned.set(true);
        }
    }
}
