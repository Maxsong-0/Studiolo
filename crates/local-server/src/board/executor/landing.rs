//! Owner-only code landing. No commit, push, ref update or synthetic candidate acceptance.
use super::{RunnerPlane, landing_candidate::Candidate, landing_store::Lease};
use serde::{Deserialize, Serialize};
use std::sync::{MutexGuard, atomic::Ordering};
use studiolo_audit_ledger::AuditPhase;
use studiolo_core::{AttemptId, TaskId};
use studiolo_runner_adapter::{CodeSnapshot, apply_code_patch};

/// Safe human response with exact candidate binding; no host path or patch body.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodeLandingResult {
    /// Owning task.
    pub task_id: TaskId,
    /// Candidate attempt.
    pub attempt_id: AttemptId,
    /// Verified binary patch digest.
    pub patch_sha256: String,
    /// Commit observed before and after application; never changed by landing.
    pub expected_head: String,
    /// Exact worktree tree required for successful landing and later acceptance.
    pub after_tree: String,
    /// True only after exact resulting tree and conflict checks.
    pub applied: bool,
    /// Whether this is a repeat of a previously persisted result.
    pub replayed: bool,
    /// Repository-relative conflict paths, if any.
    pub conflicts: Vec<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Intent {
    task: TaskId,
    attempt: AttemptId,
    patch_sha256: String,
    head: String,
    repository_identity: String,
}
/// Retain this guard through engine.review_board_candidates, without awaiting while holding it.
pub struct CodeLandingReviewGuard<'a> {
    _admission: MutexGuard<'a, ()>,
    _lease: Lease,
}
impl RunnerPlane {
    /// Caller must require the current human session and L3 before this trusted operation.
    /// Global model freeze does not block human review; revoked project permission does.
    pub fn land_candidate(
        &self,
        task: TaskId,
        expected_head: String,
        patch_sha256: String,
    ) -> Result<CodeLandingResult, String> {
        let _admission = self.admission.lock()?;
        if self.stop.load(Ordering::SeqCst) {
            return Err("Runner平面已关闭".into());
        }
        let candidate = self.landing_candidate(task, true)?;
        if patch_sha256 != candidate.metadata.sha256 {
            return Err("批准的补丁摘要不匹配候选".into());
        }
        let lease = Lease::acquire(
            &self.authority.config.state_root,
            &candidate.project.repository,
        )?;
        let intent = Intent {
            task,
            attempt: candidate.attempt,
            patch_sha256,
            head: expected_head,
            repository_identity: lease.identity.clone(),
        };
        let key = candidate.key();
        let id = format!(
            "runner-landing:{}:{}",
            candidate.attempt,
            candidate.fence.get()
        );
        let plan = candidate.plan(intent.head.clone());
        if let Some(prior) = lease.read::<Intent>(&key, "intent")? {
            verify_intent(&intent, &prior)?;
            let mut result = lease
                .read::<CodeLandingResult>(&key, "outcome")?
                .ok_or("此前落地结果未知；保留候选，禁止自动重放")?;
            verify_result(&candidate, &intent, &result)?;
            if !result.applied {
                return Err("此前落地未成功；候选和失败证据已保留，禁止自动重放".into());
            }
            if result.applied {
                plan.verify_landed()
                    .map_err(|_| "落地后的HEAD或工作树已变化，需重新核验")?;
            }
            self.authority
                .board
                .authorize(&candidate.request)
                .map_err(str::to_owned)?;
            self.authority.board.event(
                &candidate.request,
                &id,
                AuditPhase::Outcome,
                if result.applied {
                    "code_landed"
                } else {
                    "code_unlanded"
                },
            )?;
            result.replayed = true;
            return Ok(result);
        }
        // Denied CAS never writes an intent or touches the repository worktree.
        plan.verify_base()
            .map_err(|_| "HEAD或完整脏工作树与候选基线不匹配")?;
        self.authority
            .board
            .authorize(&candidate.request)
            .map_err(str::to_owned)?;
        self.authority.board.event(
            &candidate.request,
            &id,
            AuditPhase::Intent,
            "code_landing_requested",
        )?;
        lease.write(&key, "intent", &intent)?;
        // Invalidate shared metadata before effects; a crash must not reuse the old baseline.
        for profile in &self.authority.config.providers {
            CodeSnapshot::invalidate_cache(&profile.attempt_root)
                .map_err(|_| "快照缓存失效未确认；落地保持未决")?;
        }
        lease.verify_repository(&candidate.project.repository)?;
        let applied = apply_code_patch(&plan);
        lease.verify_repository(&candidate.project.repository)?;
        let result = CodeLandingResult {
            task_id: task,
            attempt_id: candidate.attempt,
            patch_sha256: candidate.metadata.sha256.clone(),
            expected_head: intent.head,
            after_tree: candidate.metadata.after_tree.clone(),
            applied: applied.as_ref().is_ok_and(|r| r.applied),
            replayed: false,
            conflicts: applied.map(|r| r.conflicts).unwrap_or_default(),
        };
        lease.write(&key, "outcome", &result)?;
        self.authority.board.event(
            &candidate.request,
            &id,
            AuditPhase::Outcome,
            if result.applied {
                "code_landed"
            } else {
                "code_unlanded"
            },
        )?;
        Ok(result)
    }
    /// Human Completed review must hold this guard until the engine records its decision.
    pub fn require_landed_candidate(
        &self,
        task: TaskId,
    ) -> Result<CodeLandingReviewGuard<'_>, String> {
        let admission = self.admission.lock()?;
        if self.stop.load(Ordering::SeqCst) {
            return Err("Runner平面已关闭".into());
        }
        let candidate = self.landing_candidate(task, false)?;
        let lease = Lease::acquire(
            &self.authority.config.state_root,
            &candidate.project.repository,
        )?;
        let key = candidate.key();
        let intent = lease
            .read::<Intent>(&key, "intent")?
            .ok_or("代码候选尚未落地")?;
        let result = lease
            .read::<CodeLandingResult>(&key, "outcome")?
            .ok_or("落地结果未知")?;
        if intent.repository_identity != lease.identity {
            return Err("落地仓库身份变化".into());
        }
        verify_result(&candidate, &intent, &result)?;
        if !result.applied {
            return Err("代码候选未成功落地，不能验收完成".into());
        }
        candidate
            .plan(result.expected_head)
            .verify_landed()
            .map_err(|_| "落地结果与当前工作树不符")?;
        Ok(CodeLandingReviewGuard {
            _admission: admission,
            _lease: lease,
        })
    }
}
fn verify_intent(expected: &Intent, actual: &Intent) -> Result<(), String> {
    if expected.task != actual.task
        || expected.attempt != actual.attempt
        || expected.patch_sha256 != actual.patch_sha256
        || expected.head != actual.head
        || expected.repository_identity != actual.repository_identity
    {
        return Err("落地请求与既有意图冲突".into());
    }
    Ok(())
}
fn verify_result(
    candidate: &Candidate,
    intent: &Intent,
    result: &CodeLandingResult,
) -> Result<(), String> {
    if intent.task != candidate.task
        || intent.attempt != candidate.attempt
        || intent.patch_sha256 != candidate.metadata.sha256
        || result.task_id != candidate.task
        || result.attempt_id != candidate.attempt
        || result.patch_sha256 != candidate.metadata.sha256
        || result.expected_head != intent.head
        || result.after_tree != candidate.metadata.after_tree
        || result.replayed
        || (result.applied && !result.conflicts.is_empty())
    {
        return Err("落地记录与候选绑定不匹配".into());
    }
    Ok(())
}
