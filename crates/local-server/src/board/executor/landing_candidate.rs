//! Resolve candidates from Control DB/Broker authority, never from request-supplied host paths.
use super::{ProjectConfig, RunnerPlane, hash};
use crate::board::requests::Request;
use studiolo_control_db::{AttemptPhase, TaskStatus};
use studiolo_core::{AttemptId, Fence, TaskId, VaultPath};
use studiolo_runner_adapter::{CodeLandingPlan, PatchMetadata};
use studiolo_vault_broker::VaultReader;

pub(super) struct Candidate {
    pub task: TaskId,
    pub attempt: AttemptId,
    pub fence: Fence,
    pub request: Request,
    pub project: ProjectConfig,
    pub metadata: PatchMetadata,
}
impl Candidate {
    pub fn key(&self) -> String {
        format!("{}-{}", self.attempt, self.fence.get())
    }
    pub fn plan(&self, head: String) -> CodeLandingPlan {
        CodeLandingPlan {
            repository: self.project.repository.clone(),
            expected_head: head,
            expected_tree: self.metadata.base_tree.clone(),
            expected_after_tree: self.metadata.after_tree.clone(),
            patch_sha256: self.metadata.sha256.clone(),
            patch_path: self.metadata.retained_path.clone(),
        }
    }
}
impl RunnerPlane {
    pub(super) fn landing_candidate(
        &self,
        task: TaskId,
        pending_only: bool,
    ) -> Result<Candidate, String> {
        let engine = self
            .authority
            .board
            .engine
            .lock()
            .map_err(|_| "黑板不可用")?;
        let view = engine
            .board_detail(task, "")
            .map_err(|_| "候选任务不可用")?
            .ok_or("候选任务不存在")?;
        if view.task.status != TaskStatus::NeedsReview
            && (pending_only || view.task.status != TaskStatus::Succeeded)
        {
            return Err("任务没有已封存待审阅代码候选".into());
        }
        if !matches!(view.task.spec.adapter.as_str(), "claude" | "codex") {
            return Err("任务不是代码Runner".into());
        }
        let attempt = view.attempt.ok_or("候选attempt缺失")?;
        if attempt.phase != AttemptPhase::Completed
            || view.task.last_attempt != Some(attempt.attempt_id)
        {
            return Err("候选attempt尚未完成封存".into());
        }
        let project = self
            .authority
            .config
            .projects
            .iter()
            .find(|p| p.project.as_str() == view.task.spec.project)
            .cloned()
            .ok_or("项目没有受信仓库映射")?;
        let profile = self
            .authority
            .config
            .providers
            .iter()
            .find(|p| p.provider.name() == view.task.spec.adapter)
            .cloned()
            .ok_or("候选adapter不在部署范围")?;
        let lineage = self
            .authority
            .board
            .identities
            .instance_lineage(&attempt.worker_id)
            .map_err(|_| "原执行身份失效")?;
        let agent_type = lineage.first().ok_or("原执行身份缺失")?.agent_type.clone();
        let request = Request {
            agent_type,
            instance_id: attempt.worker_id.clone(),
            project: project.project.clone(),
            operation: "runner.code_land".into(),
            request: serde_json::json!({"task_id":task,"attempt_id":attempt.attempt_id}),
        };
        self.authority
            .board
            .authorize(&request)
            .map_err(str::to_owned)?;
        let receipt = view.candidate_receipt.ok_or("Broker候选回执缺失")?;
        let base = format!("sources/attempts/{task}/{}", attempt.attempt_id);
        if receipt.manifest_path.as_str() != format!("{base}/manifest.json")
            || receipt.artifacts.len() != 2
        {
            return Err("候选回执绑定不匹配".into());
        }
        let read = |name: &str, limit: usize| -> Result<Vec<u8>, String> {
            let path =
                VaultPath::parse(format!("{base}/{name}")).map_err(|_| "候选逻辑路径无效")?;
            let pointer = receipt
                .artifacts
                .iter()
                .find(|a| a.path == path)
                .ok_or("候选指针缺失")?;
            let object = engine
                .broker()
                .read(&path)
                .map_err(|_| "Broker候选不可用")?;
            if object.content.len() > limit
                || object.revision != pointer.revision
                || hash(&object.content) != pointer.sha256
            {
                return Err("Broker候选revision或摘要不匹配".into());
            }
            Ok(object.content)
        };
        let patch = read("candidate.patch", 16 * 1024 * 1024)?;
        let metadata: PatchMetadata =
            serde_json::from_slice(&read("patch.json", 131072)?).map_err(|_| "补丁元数据无效")?;
        let retained = profile.retained_root.join(format!(
            "{}-{}-{}-unlanded.patch",
            profile.provider.name(),
            attempt.attempt_id,
            attempt.fence.get()
        ));
        if metadata.sha256 != hash(&patch) || metadata.retained_path != retained {
            return Err("留存补丁与Broker候选不匹配".into());
        }
        let actual = std::fs::symlink_metadata(&retained).map_err(|_| "留存补丁缺失")?;
        if !actual.is_file() || actual.len() > 16 * 1024 * 1024 {
            return Err("留存补丁类型或大小无效".into());
        }
        let bytes = std::fs::read(&retained).map_err(|_| "留存补丁不可用")?;
        if bytes != patch {
            return Err("留存补丁不等于Broker候选".into());
        }
        Ok(Candidate {
            task,
            attempt: attempt.attempt_id,
            fence: attempt.fence,
            request,
            project,
            metadata,
        })
    }
}
