//! 仅限主人的代码落地：不提交、不推送、不更新 `ref`，也不接受合成的候选。
use super::{RunnerPlane, landing_candidate::Candidate, landing_store::Lease};
use serde::{Deserialize, Serialize};
use std::sync::{MutexGuard, atomic::Ordering};
use studiolo_audit_ledger::AuditPhase;
use studiolo_core::{AttemptId, TaskId};
use studiolo_runner_adapter::{CodeSnapshot, apply_code_patch};

/// 面向人侧的安全响应，精确绑定候选；不含宿主机路径或补丁正文。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodeLandingResult {
    /// 所属任务。
    pub task_id: TaskId,
    /// 产出候选的尝试。
    pub attempt_id: AttemptId,
    /// 已核验的二进制补丁摘要。
    pub patch_sha256: String,
    /// 应用补丁前后观察到的提交；落地从不改动它。
    pub expected_head: String,
    /// 落地成功与后续验收所要求的精确工作树 `tree`。
    pub after_tree: String,
    /// 只有结果 `tree` 精确核对与冲突检查都通过后才为 `true`。
    pub applied: bool,
    /// 是否为此前已持久化结果的重放。
    pub replayed: bool,
    /// 相对仓库根的冲突路径；无冲突则为空。
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
/// 在 `engine.review_board_candidates` 全程持有此守卫，持有期间不得 `await`。
pub struct CodeLandingReviewGuard<'a> {
    _admission: MutexGuard<'a, ()>,
    _lease: Lease,
}
impl RunnerPlane {
    /// 调用方须在执行这一受信任操作之前要求当前人侧会话与 L3。
    /// 全局模型冻结不阻止人工审阅；项目权限被撤销则会阻止。
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
        // `CAS` 被拒时绝不写入意图，也不触碰仓库工作树。
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
        // 产生副作用之前先作废共享元数据；崩溃后不得沿用旧基线。
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
    /// 人工 `Completed` 审阅须一直持有此守卫，直到引擎记下审阅结论。
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
