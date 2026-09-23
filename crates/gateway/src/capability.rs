//! capability 与实时权威：短期授权凭证、attempt 状态与 guard、时钟、三层权限上限。

use crate::AttemptGuardError;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::Mutex};
use studiolo_core::{AttemptId, Fence, PermissionSet, TaskId};

/// runner 为某一次 attempt 出示的短期授权凭证。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobCapability {
    task_id: TaskId,
    attempt_id: AttemptId,
    fence: Fence,
    allowed_tools: PermissionSet,
    expires_at_unix_seconds: i64,
}

impl JobCapability {
    /// 构造已校验 capability 在 M0 进程内的表示。
    #[must_use]
    pub fn new(
        task_id: TaskId,
        attempt_id: AttemptId,
        fence: Fence,
        allowed_tools: PermissionSet,
        expires_at_unix_seconds: i64,
    ) -> Self {
        Self {
            task_id,
            attempt_id,
            fence,
            allowed_tools,
            expires_at_unix_seconds,
        }
    }

    /// 返回绑定的任务。
    #[must_use]
    pub fn task_id(&self) -> TaskId {
        self.task_id
    }

    /// 返回绑定的 attempt。
    #[must_use]
    pub fn attempt_id(&self) -> AttemptId {
        self.attempt_id
    }

    /// 返回绑定的 fencing token。
    #[must_use]
    pub fn fence(&self) -> Fence {
        self.fence
    }

    /// 返回该 capability 的工具上限。
    #[must_use]
    pub fn allowed_tools(&self) -> &PermissionSet {
        &self.allowed_tools
    }

    /// 返回到期时间（自 Unix 纪元起的秒数）。
    #[must_use]
    pub fn expires_at_unix_seconds(&self) -> i64 {
        self.expires_at_unix_seconds
    }
}

/// [`AttemptGuard`] 为某次 attempt 返回的最新状态。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AttemptSnapshot {
    fence: Fence,
    cancelled: bool,
}

impl AttemptSnapshot {
    /// 创建活动中的 attempt 状态。
    #[must_use]
    pub fn active(fence: Fence) -> Self {
        Self {
            fence,
            cancelled: false,
        }
    }

    /// 创建已取消的 attempt 状态。
    #[must_use]
    pub fn cancelled(fence: Fence) -> Self {
        Self {
            fence,
            cancelled: true,
        }
    }

    /// 返回当前的 fencing token。
    #[must_use]
    pub fn fence(self) -> Fence {
        self.fence
    }

    /// 返回 capability 的使用是否已被取消。
    #[must_use]
    pub fn is_cancelled(self) -> bool {
        self.cancelled
    }
}

/// 用于 fence 与取消检查的实时、无缓存权威来源。
pub trait AttemptGuard: Send + Sync {
    /// 加载精确 task/attempt 组合的当前状态。
    fn inspect(
        &self,
        task_id: TaskId,
        attempt_id: AttemptId,
    ) -> Result<AttemptSnapshot, AttemptGuardError>;
}

/// 可变的内存 attempt 权威来源，供契约测试与重启测试使用。
#[derive(Debug, Default)]
pub struct MemoryAttemptGuard {
    attempts: Mutex<BTreeMap<(TaskId, AttemptId), AttemptSnapshot>>,
}

impl MemoryAttemptGuard {
    /// 创建一个空的 guard。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 插入或更新某次 attempt 的当前状态。
    pub fn set(
        &self,
        task_id: TaskId,
        attempt_id: AttemptId,
        snapshot: AttemptSnapshot,
    ) -> Result<(), AttemptGuardError> {
        self.attempts
            .lock()
            .map_err(|_| AttemptGuardError::Unavailable)?
            .insert((task_id, attempt_id), snapshot);
        Ok(())
    }
}

impl AttemptGuard for MemoryAttemptGuard {
    fn inspect(
        &self,
        task_id: TaskId,
        attempt_id: AttemptId,
    ) -> Result<AttemptSnapshot, AttemptGuardError> {
        self.attempts
            .lock()
            .map_err(|_| AttemptGuardError::Unavailable)?
            .get(&(task_id, attempt_id))
            .copied()
            .ok_or(AttemptGuardError::UnknownAttempt)
    }
}

/// 时钟边界：用于评估 capability 是否到期，使测试无需 sleep。
pub trait Clock: Send + Sync {
    /// 返回自 Unix 纪元起的整秒数。
    fn now_unix_seconds(&self) -> i64;
}

/// 供 M0 契约测试使用的确定性时钟。
#[derive(Debug)]
pub struct ManualClock {
    now: std::sync::atomic::AtomicI64,
}

impl ManualClock {
    /// 创建固定在 `now_unix_seconds` 的时钟。
    #[must_use]
    pub fn new(now_unix_seconds: i64) -> Self {
        Self {
            now: std::sync::atomic::AtomicI64::new(now_unix_seconds),
        }
    }

    /// 修改当前时刻。
    pub fn set(&self, now_unix_seconds: i64) {
        self.now
            .store(now_unix_seconds, std::sync::atomic::Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn now_unix_seconds(&self) -> i64 {
        self.now.load(std::sync::atomic::Ordering::SeqCst)
    }
}

/// 相互独立施加的部署、策略与角色三层上限。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PermissionLayers {
    deployment: PermissionSet,
    policy: PermissionSet,
    role: PermissionSet,
}

impl PermissionLayers {
    /// 创建 Gateway 自有的三层权限上限。
    #[must_use]
    pub fn new(deployment: PermissionSet, policy: PermissionSet, role: PermissionSet) -> Self {
        Self {
            deployment,
            policy,
            role,
        }
    }

    pub(crate) fn effective(&self, capability: &JobCapability) -> PermissionSet {
        self.deployment
            .intersect(&self.policy)
            .intersect(&self.role)
            .intersect(capability.allowed_tools())
    }
}
