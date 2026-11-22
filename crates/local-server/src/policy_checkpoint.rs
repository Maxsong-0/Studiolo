//! 策略参与者：先排空人侧完整操作，再阻断所有进程的策略事务。
//! 上游AgentStore参与者必须先排空接入方完整调用；不能单独据此宣称全系统已冻结。
use std::{
    path::PathBuf,
    sync::{Arc, Mutex, MutexGuard, TryLockError},
    thread,
    time::{Duration, Instant},
};
use studiolo_core::checkpoint::{
    CheckpointError, CheckpointOwner, CheckpointParticipant, DrainProof, FreezeReceipt,
    FreezeRequest, OwnerAssembly, SealedOwnerCheckpoint,
};
use studiolo_policy::{PolicyCheckpointHandle, PolicyError};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

const OWNER: CheckpointOwner = CheckpointOwner::Policy;

struct Present {
    handle: PolicyCheckpointHandle,
    permits: Arc<Semaphore>,
    target: PathBuf,
}
struct Lease {
    request: FreezeRequest,
    deadline: Instant,
    /// 冻结期间占满策略 HTTP 的许可；放开之后是 None，HTTP 照常处理（BUG-045）。
    permits: Option<OwnedSemaphorePermit>,
}

/// 必须由实际PolicyPlane构造；不会用另一条源路径与许可池错配。
pub struct PolicyParticipant {
    global_deadline: Option<Instant>,
    inner: Option<Present>,
    lease: Mutex<Option<Lease>>,
}
impl PolicyParticipant {
    /// 总协调器的原始截止时间；后续 freeze/export 不得刷新整个窗口预算。
    pub(crate) fn before(mut self, deadline: Instant) -> Self {
        self.global_deadline = Some(deadline);
        self
    }

    pub(crate) fn present(
        handle: PolicyCheckpointHandle,
        permits: Arc<Semaphore>,
        target: PathBuf,
    ) -> Self {
        Self {
            global_deadline: None,
            inner: Some(Present {
                handle,
                permits,
                target,
            }),
            lease: Mutex::new(None),
        }
    }

    /// 只用于该部署确实没有策略平面；实际存在的库不能以此跳过。
    pub fn absent() -> Self {
        Self {
            global_deadline: None,
            inner: None,
            lease: Mutex::new(None),
        }
    }

    /// 放开写屏障和 HTTP 许可（BUG-045）：定格的读连接留着，之后的导出读的仍是冻结那一刻、受 `deadline` 约束。
    /// 只能放开一次；没冻结就拒绝。
    pub(crate) fn release(&self, deadline: Instant) -> Result<(), CheckpointError> {
        let inner = self
            .inner
            .as_ref()
            .ok_or(CheckpointError::OwnerUnavailable { owner: OWNER })?;
        let mut guard = self.lock_until(deadline)?;
        let lease = guard
            .as_mut()
            .filter(|lease| lease.permits.is_some())
            .ok_or(CheckpointError::NotDrained { owner: OWNER })?;
        inner.handle.release(deadline).map_err(problem)?;
        lease.permits = None;
        lease.deadline = deadline;
        Ok(())
    }

    fn lock_until(
        &self,
        deadline: Instant,
    ) -> Result<MutexGuard<'_, Option<Lease>>, CheckpointError> {
        loop {
            if Instant::now() >= deadline {
                return Err(CheckpointError::FreezeTimeout { owner: OWNER });
            }
            match self.lease.try_lock() {
                Ok(guard) => return Ok(guard),
                Err(TryLockError::Poisoned(_)) => {
                    return Err(CheckpointError::OwnerUnavailable { owner: OWNER });
                }
                Err(TryLockError::WouldBlock) => pause(deadline),
            }
        }
    }
}
impl CheckpointParticipant for PolicyParticipant {
    fn owner(&self) -> CheckpointOwner {
        OWNER
    }
    fn assembly(&self) -> OwnerAssembly {
        if self.inner.is_some() {
            OwnerAssembly::Present
        } else {
            OwnerAssembly::Absent
        }
    }
    fn freeze(&self, request: &FreezeRequest) -> Result<FreezeReceipt, CheckpointError> {
        let inner = self
            .inner
            .as_ref()
            .ok_or(CheckpointError::OwnerUnavailable { owner: OWNER })?;
        let deadline = Instant::now() + Duration::from_millis(u64::from(request.budget.millis()));
        let deadline = self
            .global_deadline
            .map_or(deadline, |global| global.min(deadline));
        let mut lease = self.lock_until(deadline)?;
        if lease.is_some() {
            return Err(CheckpointError::NotDrained { owner: OWNER });
        }
        let permits = loop {
            if Instant::now() >= deadline {
                return Err(CheckpointError::FreezeTimeout { owner: OWNER });
            }
            match inner
                .permits
                .clone()
                .try_acquire_many_owned(crate::policy_http::POLICY_CONCURRENCY)
            {
                Ok(permits) => break permits,
                Err(tokio::sync::TryAcquireError::Closed) => {
                    return Err(CheckpointError::OwnerUnavailable { owner: OWNER });
                }
                Err(tokio::sync::TryAcquireError::NoPermits) => pause(deadline),
            }
        };
        inner.handle.freeze(deadline).map_err(problem)?;
        *lease = Some(Lease {
            request: request.clone(),
            deadline,
            permits: Some(permits),
        });
        Ok(FreezeReceipt {
            owner: OWNER,
            drain: DrainProof {
                in_flight_operations: 0,
                prepared_readable: false,
                background_delivery_pending: 0,
                local_maintenance_writers_blocked: true,
                implicit_read_writes_blocked: true,
            },
        })
    }
    fn export(&self, request: &FreezeRequest) -> Result<SealedOwnerCheckpoint, CheckpointError> {
        let inner = self
            .inner
            .as_ref()
            .ok_or(CheckpointError::OwnerUnavailable { owner: OWNER })?;
        let deadline = Instant::now() + Duration::from_millis(u64::from(request.budget.millis()));
        let deadline = self
            .global_deadline
            .map_or(deadline, |global| global.min(deadline));
        // 放开之后冻结窗口的截止时间早过了：不拿它等锁，拿得到就直接用。
        let guard = match self.lease.try_lock() {
            Ok(guard) => guard,
            Err(TryLockError::Poisoned(_)) => {
                return Err(CheckpointError::OwnerUnavailable { owner: OWNER });
            }
            Err(TryLockError::WouldBlock) => self.lock_until(deadline)?,
        };
        let lease = guard
            .as_ref()
            .filter(|lease| lease.request == *request)
            .ok_or(CheckpointError::NotDrained { owner: OWNER })?;
        // 放开之后只受放开时给的截止时间约束；没放开照旧受冻结窗口约束。
        let deadline = if lease.permits.is_none() {
            lease.deadline
        } else {
            lease.deadline.min(deadline)
        };
        inner
            .handle
            .export(&inner.target, deadline)
            .map_err(problem)
    }
    fn thaw(&self) -> Result<(), CheckpointError> {
        let (mut lease, poisoned) = match self.lease.lock() {
            Ok(guard) => (guard, false),
            Err(error) => (error.into_inner(), true),
        };
        let result = self
            .inner
            .as_ref()
            .map_or(Ok(()), |inner| inner.handle.thaw().map_err(problem));
        // 先释放SQLite屏障，再让HTTP恢复；错误路径同样释放所有许可。
        lease.take();
        if poisoned {
            return Err(CheckpointError::OwnerUnavailable { owner: OWNER });
        }
        result
    }
}
impl Drop for PolicyParticipant {
    fn drop(&mut self) {
        let _ = self.thaw();
    }
}
fn pause(deadline: Instant) {
    thread::sleep(
        deadline
            .saturating_duration_since(Instant::now())
            .min(Duration::from_millis(10)),
    );
}
fn problem(error: PolicyError) -> CheckpointError {
    match error {
        PolicyError::Busy => CheckpointError::FreezeTimeout { owner: OWNER },
        PolicyError::CheckpointTarget | PolicyError::UnsafeStorage | PolicyError::CorruptStore => {
            CheckpointError::Unverified { owner: OWNER }
        }
        _ => CheckpointError::OwnerUnavailable { owner: OWNER },
    }
}

#[cfg(test)]
#[path = "../tests/policy_checkpoint/private.rs"]
mod tests;
