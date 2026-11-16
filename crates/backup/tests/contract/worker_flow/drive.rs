//! 常驻循环的一步：决策只看本地位置和来源是否还有效，每条路都是原状态机已有的。
use super::*;
use std::cell::Cell;
use std::num::NonZeroU64;
use std::sync::atomic::Ordering;
use studiolo_audit_ledger::AuditLedger;
use studiolo_backup::worker::final_manifest::{
    FinalManifestDraft, FinalManifestLookup, FinalManifestReader, FinalManifestWriter,
};
use studiolo_backup::worker::job::{
    Advance, JobPorts, JobPosition, PortUnavailable, RemoteProbe, Transfer, backup_intent,
};
use studiolo_backup::worker::restic::{ResticError, ResticInvocation, ResticOutcome, ResticRunner};
use studiolo_backup::worker::state::{JobKey, UnknownStage};

const TARGET: BackupTargetKind = BackupTargetKind::DedicatedAppendOnlyTarget;

fn pending(world: &World) -> JobKey {
    let key = JobKey {
        sequence: NonZeroU64::new(5).unwrap(),
        checkpoint_id: CheckpointId::parse("cp-0001").unwrap(),
    };
    // 生产者一侧做的事：封存之前把原意图写进账本。
    world
        .ledger
        .append(backup_intent(&key, TARGET).unwrap())
        .unwrap();
    key
}

/// 记下被叫了几次；`broken` 时像进程中途断掉一样返回错误。
struct Uploader {
    broken: bool,
    calls: Cell<u32>,
}
impl ResticRunner for Uploader {
    fn run(&self, _: &ResticInvocation) -> Result<ResticOutcome, ResticError> {
        self.calls.set(self.calls.get() + 1);
        if self.broken {
            return Err(ResticError::ExecutionFailed);
        }
        Ok(ResticOutcome::Exited {
            status_zero: true,
            reported_objects: 3,
        })
    }
}
struct Ports<'a> {
    world: &'a World,
    live: bool,
    openable: bool,
    uploader: Uploader,
    probe: StubProbe,
    opened: Cell<u32>,
}
impl<'a> Ports<'a> {
    fn new(world: &'a World) -> Self {
        Self {
            world,
            live: true,
            openable: true,
            uploader: Uploader {
                broken: false,
                calls: Cell::new(0),
            },
            probe: StubProbe::present(2_000),
            opened: Cell::new(0),
        }
    }
}
struct Opened<'a>(&'a Ports<'a>);
impl Transfer for Opened<'_> {
    fn sealed(&self) -> &SealedCheckpoint {
        &self.0.world.seal
    }
    fn runner(&self) -> &dyn ResticRunner {
        &self.0.uploader
    }
    fn probe(&self) -> &dyn RemoteProbe {
        &self.0.probe
    }
    fn finalizer(&self) -> &dyn FinalManifestWriter {
        &self.0.world.finalizer
    }
}
struct Reader<'a>(&'a StubFinal);
impl FinalManifestReader for Reader<'_> {
    fn read_final(&self, draft: &FinalManifestDraft) -> Result<FinalManifestLookup, ResticError> {
        self.0.read_final(draft)
    }
}
impl JobPorts for Ports<'_> {
    fn source_live(&self) -> bool {
        self.live
    }
    fn transfer(&self) -> Result<Box<dyn Transfer + '_>, PortUnavailable> {
        if !self.openable {
            return Err(PortUnavailable);
        }
        self.opened.set(self.opened.get() + 1);
        Ok(Box::new(Opened(self)))
    }
    fn final_reader(&self) -> Result<Box<dyn FinalManifestReader + '_>, PortUnavailable> {
        if !self.openable {
            return Err(PortUnavailable);
        }
        Ok(Box::new(Reader(&self.world.finalizer)))
    }
}

#[test]
fn a_fresh_pending_key_is_accepted_and_run_to_success_in_one_step() {
    let world = World::new();
    let worker = world.worker(TARGET);
    let key = pending(&world);
    let ports = Ports::new(&world);
    assert_eq!(
        worker.advance(&key, &ports).unwrap(),
        Advance::Outcome(JobOutcome::Succeeded)
    );
    assert_eq!(worker.position(&key).unwrap(), JobPosition::Finished);
    assert_eq!((ports.opened.get(), ports.uploader.calls.get()), (1, 1));
    assert_eq!(world.ledger.records().len(), 2, "原意图加成功结果");
    // 本地已经收口，生产者却还把它列为待办：拒绝，不重做任何事。
    assert_eq!(
        worker.advance(&key, &ports).err(),
        Some(WorkerError::EvidenceMismatch)
    );
    assert_eq!((ports.opened.get(), ports.uploader.calls.get()), (1, 1));
}

#[test]
fn an_expired_source_is_failed_without_reopening_it() {
    // 还没接收就已经过期：照样先登记原键，再记失败，不去开封存。
    let world = World::new();
    let worker = world.worker(TARGET);
    let key = pending(&world);
    let mut ports = Ports::new(&world);
    ports.live = false;
    assert_eq!(
        worker.advance(&key, &ports).unwrap(),
        Advance::Outcome(JobOutcome::Failed(FailureReason::SourceExpired))
    );
    assert_eq!(ports.opened.get(), 0);
    assert_eq!(worker.position(&key).unwrap(), JobPosition::Finished);

    // 上传中途断掉，之后来源过期：不重传，记失败。
    let world = World::new();
    let worker = world.worker(TARGET);
    let key = pending(&world);
    let mut ports = Ports::new(&world);
    ports.uploader.broken = true;
    assert_eq!(
        worker.advance(&key, &ports).unwrap(),
        Advance::Outcome(JobOutcome::ResultUnknown(UnknownStage::Uploading))
    );
    assert_eq!(worker.position(&key).unwrap(), JobPosition::Interrupted);
    ports.live = false;
    assert_eq!(
        worker.advance(&key, &ports).unwrap(),
        Advance::Outcome(JobOutcome::Failed(FailureReason::SourceExpired))
    );
    assert_eq!((ports.opened.get(), ports.uploader.calls.get()), (1, 1));
}

#[test]
fn an_interrupted_upload_is_reconciled_read_only_and_never_uploaded_again() {
    let world = World::new();
    let worker = world.worker(TARGET);
    let key = pending(&world);
    let mut ports = Ports::new(&world);
    ports.uploader.broken = true;
    worker.advance(&key, &ports).unwrap();
    // 进程重启：执行器恢复正常也不许再传一遍，只沿原键核对远端。
    ports.uploader.broken = false;
    let restarted = world.worker(TARGET);
    restarted.recover_interrupted().unwrap();
    assert_eq!(
        restarted.advance(&key, &ports).unwrap(),
        Advance::Outcome(JobOutcome::Succeeded)
    );
    assert_eq!(ports.uploader.calls.get(), 1, "上传只发生过那一次");
    assert_eq!(ports.opened.get(), 2);
}

#[test]
fn ports_that_cannot_be_assembled_defer_and_leave_the_job_where_it_was() {
    let world = World::new();
    let worker = world.worker(TARGET);
    let key = pending(&world);
    let mut ports = Ports::new(&world);
    ports.openable = false;
    assert_eq!(worker.advance(&key, &ports).unwrap(), Advance::Deferred);
    assert_eq!(worker.position(&key).unwrap(), JobPosition::Queued);
    assert_eq!(world.ledger.records().len(), 1, "除了接收原键什么都没发生");
    ports.openable = true;
    assert_eq!(
        worker.advance(&key, &ports).unwrap(),
        Advance::Outcome(JobOutcome::Succeeded)
    );
}

#[test]
fn a_persisted_final_draft_is_finished_after_expiry_and_a_lost_failure_is_backfilled() {
    // 最终清单的回执丢了：之后来源过期也照样只读核对并收口，不需要正文。
    let world = World::new();
    let worker = world.worker(TARGET);
    let key = pending(&world);
    let mut ports = Ports::new(&world);
    *world.finalizer.lose_ack.lock().unwrap() = true;
    assert_eq!(
        worker.advance(&key, &ports).unwrap(),
        Advance::Outcome(JobOutcome::ResultUnknown(UnknownStage::FinalManifestAck))
    );
    assert_eq!(worker.position(&key).unwrap(), JobPosition::Finalizing);
    ports.live = false;
    assert_eq!(
        worker.advance(&key, &ports).unwrap(),
        Advance::Outcome(JobOutcome::Succeeded)
    );
    assert_eq!(ports.opened.get(), 1, "核对最终清单不重开正文");
    assert_eq!(world.finalizer.commits.load(Ordering::SeqCst), 1);

    // 失败结果没写进账本：下一步只补记，不做别的。
    let world = World::new();
    let worker = world.worker(TARGET);
    let key = pending(&world);
    let mut ports = Ports::new(&world);
    ports.live = false;
    world.ledger.fail(AuditPhase::Outcome);
    assert!(matches!(
        worker.advance(&key, &ports),
        Err(WorkerError::Audit(_))
    ));
    assert_eq!(worker.position(&key).unwrap(), JobPosition::FailurePending);
    world.ledger.recover();
    assert_eq!(
        worker.advance(&key, &ports).unwrap(),
        Advance::Outcome(JobOutcome::Failed(FailureReason::SourceExpired))
    );
    assert_eq!(ports.opened.get(), 0);
}
