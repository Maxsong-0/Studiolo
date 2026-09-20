//! Fresh canonical claim, policy, lineage and classification checks for every relay boundary.
use super::{Binding, RunnerConfig};
use crate::board::{BoardPlane, requests::Request};
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use studiolo_control_db::AttemptPhase;
use studiolo_core::{AttemptId, TaskId};
use studiolo_gateway::{
    AttemptGuard, AttemptGuardError, AttemptSnapshot, RelayError, RelayGrant, RelayPolicyGuard,
};

pub(super) struct Authority {
    pub board: Arc<BoardPlane>,
    pub config: RunnerConfig,
    pub frozen: AtomicBool,
    pub bindings: Mutex<BTreeMap<AttemptId, Binding>>,
}
impl AttemptGuard for Authority {
    fn inspect(
        &self,
        task: TaskId,
        attempt: AttemptId,
    ) -> Result<AttemptSnapshot, AttemptGuardError> {
        let engine = self
            .board
            .engine
            .lock()
            .map_err(|_| AttemptGuardError::Unavailable)?;
        let a = engine
            .control_db()
            .attempt(task, attempt)
            .map_err(|_| AttemptGuardError::Unavailable)?
            .ok_or(AttemptGuardError::UnknownAttempt)?;
        let now = jiff::Timestamp::now().as_millisecond();
        if self.frozen.load(Ordering::SeqCst) || a.phase == AttemptPhase::CancelRequested {
            return Ok(AttemptSnapshot::cancelled(a.fence));
        }
        if a.runner_ref_json.is_none()
            || !engine
                .control_db()
                .is_attempt_active_at(task, attempt, a.fence, now)
                .map_err(|_| AttemptGuardError::Unavailable)?
        {
            return Err(AttemptGuardError::UnknownAttempt);
        }
        Ok(AttemptSnapshot::active(a.fence))
    }
}
impl RelayPolicyGuard for Authority {
    fn inspect(&self, grant: &RelayGrant, now: i64) -> Result<(), RelayError> {
        if self.frozen.load(Ordering::SeqCst) {
            return Err(RelayError::Frozen);
        }
        let binding = self
            .bindings
            .lock()
            .map_err(|_| RelayError::Storage)?
            .get(&grant.capability.attempt_id())
            .cloned()
            .ok_or(RelayError::Denied)?;
        if binding.grant.as_ref() != Some(grant) {
            return Err(RelayError::Denied);
        }
        let r = Request {
            agent_type: binding.agent_type.clone(),
            instance_id: binding.instance_id.clone(),
            project: binding.project.clone(),
            operation: "runner.relay".into(),
            request: serde_json::json!({}),
        };
        let current = self.board.authorize(&r).map_err(|_| RelayError::Denied)?;
        if current.hash != grant.policy_hash {
            return Err(RelayError::Denied);
        }
        let engine = self.board.engine.lock().map_err(|_| RelayError::Storage)?;
        let a = engine
            .control_db()
            .attempt(grant.capability.task_id(), grant.capability.attempt_id())
            .map_err(|_| RelayError::Storage)?
            .ok_or(RelayError::Denied)?;
        if a.snapshot_id != grant.snapshot_id
            || a.worker_id != binding.instance_id
            || a.lease_until_ms <= now.saturating_mul(1000)
            || !engine
                .owns_board_claim(
                    a.task_id,
                    a.attempt_id,
                    a.fence,
                    &binding.instance_id,
                    now.saturating_mul(1000),
                )
                .map_err(|_| RelayError::Denied)?
        {
            return Err(RelayError::Denied);
        }
        // The private configuration is immutable for this process; every revision/classification is exact.
        let configured = self
            .config
            .projects
            .iter()
            .find(|p| p.project == binding.project)
            .ok_or(RelayError::Denied)?;
        if configured.retain_until_unix_seconds <= now {
            return Err(RelayError::Egress);
        }
        Ok(())
    }
}
pub(super) struct SystemClock;
impl studiolo_gateway::Clock for SystemClock {
    fn now_unix_seconds(&self) -> i64 {
        jiff::Timestamp::now().as_second()
    }
}
