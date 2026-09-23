//! Worker 侧共享假实现：内存状态存储、假 restic、假远端探针与只在指定 phase 失败的审计存储。
//!
//! 这些假实现只用于驱动状态与判定，绝不假装真实备份目标存在。

use std::sync::Mutex;

use studiolo_audit_ledger::{
    AuditError, AuditEvent, AuditFilter, AuditLedger, AuditPhase, AuditRecord, MemoryAuditLedger,
};
use studiolo_backup::checkpoint::manifest::CheckpointManifest;
use studiolo_backup::store::model::{BackupStateStore, PersistedState, StoreError};
use studiolo_backup::worker::job::RemoteProbe;
use studiolo_backup::worker::restic::{
    ResticError, ResticInvocation, ResticOutcome, ResticRunner, SnapshotRef,
};
use studiolo_backup::worker::state::{JobKey, RemoteVerification};
use studiolo_core::checkpoint::{CheckpointId, InstantMillis};

/// 内存状态存储；用来把 Worker 的持久语义与真实 IO 分开验证。
#[derive(Default)]
pub struct MemoryStore {
    state: Mutex<Option<PersistedState>>,
    manifest: Mutex<Option<CheckpointManifest>>,
    pub fail_manifest_commit: Mutex<Option<StoreError>>,
}

impl BackupStateStore for MemoryStore {
    fn load_state(&self) -> Result<Option<PersistedState>, StoreError> {
        Ok(self.state.lock().expect("state lock").clone())
    }

    fn save_state(&self, state: &PersistedState) -> Result<(), StoreError> {
        state.validate()?;
        *self.state.lock().expect("state lock") = Some(state.clone());
        Ok(())
    }

    fn commit_final_manifest(&self, manifest: &CheckpointManifest) -> Result<(), StoreError> {
        if let Some(error) = *self.fail_manifest_commit.lock().expect("fail lock") {
            return Err(error);
        }
        *self.manifest.lock().expect("manifest lock") = Some(manifest.clone());
        Ok(())
    }

    fn load_final_manifest(&self) -> Result<Option<CheckpointManifest>, StoreError> {
        Ok(self.manifest.lock().expect("manifest lock").clone())
    }
}

/// 假 restic 执行器：按脚本返回退出码或明确未验证。
pub struct StubRunner {
    pub outcome: ResticOutcome,
    pub seen: Mutex<Vec<Vec<String>>>,
}

impl StubRunner {
    pub fn exiting(status_zero: bool) -> Self {
        Self {
            outcome: ResticOutcome::Exited {
                status_zero,
                reported_objects: 3,
            },
            seen: Mutex::new(Vec::new()),
        }
    }

    pub fn unverified(reason: studiolo_backup::worker::restic::UnverifiedReason) -> Self {
        Self {
            outcome: ResticOutcome::Unverified { reason },
            seen: Mutex::new(Vec::new()),
        }
    }
}

impl ResticRunner for StubRunner {
    fn run(&self, invocation: &ResticInvocation) -> Result<ResticOutcome, ResticError> {
        self.seen
            .lock()
            .expect("seen lock")
            .push(invocation.argv().to_vec());
        Ok(self.outcome)
    }
}

/// 假远端探针：按作业键返回是否存在，以及校验结果。
pub struct StubProbe {
    pub located: Option<SnapshotRef>,
    pub verification: RemoteVerification,
    pub probed: Mutex<Vec<u64>>,
}

impl StubProbe {
    pub fn present(verified_at: i64) -> Self {
        Self {
            located: Some(SnapshotRef::parse("0a1b2c3d").expect("snapshot")),
            verification: RemoteVerification {
                objects_complete: true,
                checksum_verified: true,
                paired_checkpoint_reference: true,
                verified_at: InstantMillis::from_millis(verified_at),
            },
            probed: Mutex::new(Vec::new()),
        }
    }

    pub fn absent() -> Self {
        Self {
            located: None,
            verification: RemoteVerification {
                objects_complete: false,
                checksum_verified: false,
                paired_checkpoint_reference: false,
                verified_at: InstantMillis::from_millis(0),
            },
            probed: Mutex::new(Vec::new()),
        }
    }
}

impl RemoteProbe for StubProbe {
    fn locate(&self, key: &JobKey) -> Result<Option<SnapshotRef>, ResticError> {
        self.probed
            .lock()
            .expect("probe lock")
            .push(key.sequence.get());
        Ok(self.located.clone())
    }

    fn verify(
        &self,
        _snapshot: &SnapshotRef,
        _checkpoint_id: &CheckpointId,
    ) -> Result<RemoteVerification, ResticError> {
        Ok(self.verification)
    }
}

/// 只在指定 phase 上失败的审计存储。
pub struct PickyLedger {
    inner: MemoryAuditLedger,
    pub failing_phase: Mutex<Option<AuditPhase>>,
}

impl PickyLedger {
    pub fn new() -> Self {
        Self {
            inner: MemoryAuditLedger::default(),
            failing_phase: Mutex::new(None),
        }
    }

    pub fn fail(&self, phase: AuditPhase) {
        *self.failing_phase.lock().expect("phase lock") = Some(phase);
    }

    pub fn recover(&self) {
        *self.failing_phase.lock().expect("phase lock") = None;
    }

    pub fn records(&self) -> Vec<AuditRecord> {
        self.inner
            .query(&AuditFilter::default())
            .expect("query audit")
    }
}

impl AuditLedger for PickyLedger {
    fn append(&self, event: AuditEvent) -> Result<AuditRecord, AuditError> {
        if *self.failing_phase.lock().expect("phase lock") == Some(event.phase()) {
            return Err(AuditError::Unavailable);
        }
        self.inner.append(event)
    }

    fn query(&self, filter: &AuditFilter) -> Result<Vec<AuditRecord>, AuditError> {
        self.inner.query(filter)
    }
}
