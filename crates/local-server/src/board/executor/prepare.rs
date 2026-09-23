//! 只依据经认证的认领与私有配置构造不可变的执行输入。
use super::{ProjectConfig, ProviderConfig, RunnerPlane, StartRequest, hash};
use studiolo_core::{PermissionSet, ToolId, VaultPath};
use studiolo_gateway::{JobCapability, ModelEgress, RelayGrant, RelayProvider, RelaySource};
use studiolo_runner_adapter::{
    CliProvider, ExecutionSpec, LinuxIsolation, RealRunner, RunInput, RunSpec, RunnerLaunchConfig,
    SourceEgress,
};
use studiolo_task_engine::BoardClaim;
use studiolo_vault_broker::{SnapshotId, SnapshotReader};

pub(super) fn provider(kind: CliProvider) -> RelayProvider {
    match kind {
        CliProvider::Claude => RelayProvider::Anthropic,
        CliProvider::Codex => RelayProvider::OpenAi,
    }
}
fn egress(label: &ModelEgress) -> SourceEgress {
    match label {
        ModelEgress::Deny => SourceEgress::Deny,
        ModelEgress::LocalOnly => SourceEgress::LocalOnly,
        ModelEgress::Providers(values) => SourceEgress::Providers(
            values
                .iter()
                .map(|v| match v {
                    RelayProvider::Anthropic => CliProvider::Claude,
                    RelayProvider::OpenAi => CliProvider::Codex,
                })
                .collect(),
        ),
    }
}
impl RunnerPlane {
    pub(super) fn probe_runner(&self, profile: &ProviderConfig) -> Result<RealRunner, String> {
        let project = self
            .authority
            .config
            .projects
            .first()
            .ok_or("没有Runner项目映射")?;
        Ok(RealRunner::new(
            self.runner_config(
                profile,
                project,
                self.authority
                    .config
                    .state_root
                    .join("unused-probe-capability"),
            ),
        ))
    }
    pub(super) fn runner_config(
        &self,
        p: &ProviderConfig,
        project: &ProjectConfig,
        ticket: std::path::PathBuf,
    ) -> RunnerLaunchConfig {
        RunnerLaunchConfig {
            provider: p.provider,
            cli_sha256: p.cli_sha256.clone(),
            isolation: LinuxIsolation {
                runtime_root: p.runtime_root.clone(),
                bubblewrap: p.bubblewrap.clone(),
                systemd_run: p.systemd_run.clone(),
                gateway_socket: self.socket.path().to_path_buf(),
            },
            source_checkout: project.repository.clone(),
            attempt_root: p.attempt_root.clone(),
            retained_root: p.retained_root.clone(),
            capability_file: ticket,
        }
    }
    pub(super) fn prepare(
        &self,
        claim: &BoardClaim,
        r: &StartRequest,
        policy_hash: &str,
    ) -> Result<(RunSpec, RelayGrant, ProviderConfig, ProjectConfig), String> {
        let task = &claim.task.task;
        let profile = self
            .authority
            .config
            .providers
            .iter()
            .find(|p| p.provider.name() == task.spec.adapter)
            .cloned()
            .ok_or("任务adapter未部署")?;
        let project = self
            .authority
            .config
            .projects
            .iter()
            .find(|p| p.project.as_str() == task.spec.project)
            .cloned()
            .ok_or("项目未授Runner范围")?;
        if !project.allow_full_artifact_backup || !project.allow_artifact_desktop_sync {
            return Err("代码产物尚未批准canonical备份与桌面同步；本部署不能提升raw或patch".into());
        }
        let policy = self
            .authority
            .config
            .policies
            .iter()
            .find(|p| p.hash == policy_hash && p.role == task.spec.role)
            .ok_or("Runner角色或effective策略未授权")?;
        if !self
            .authority
            .config
            .deployment_tools
            .iter()
            .any(|t| t == "model.relay")
            || !policy.policy_tools.iter().any(|t| t == "model.relay")
            || !policy.role_tools.iter().any(|t| t == "model.relay")
        {
            return Err("模型中继四层交集拒绝".into());
        }
        let snapshot_id: SnapshotId =
            serde_json::from_value(serde_json::json!(claim.attempt.snapshot_id))
                .map_err(|_| "快照ID无效")?;
        let snapshot = self
            .authority
            .board
            .engine
            .lock()
            .map_err(|_| "黑板不可用")?
            .broker()
            .load_snapshot(&snapshot_id)
            .map_err(|_| "快照不可用")?;
        let mut inputs = Vec::new();
        let mut labels = Vec::new();
        let mut sources = Vec::new();
        for file in snapshot.files {
            if !crate::board::auth::within(&file.path, &project.project) {
                return Err("输入不在显式项目白名单".into());
            }
            let label = project
                .sources
                .iter()
                .find(|s| s.path == file.path)
                .ok_or("快照存在未分类来源")?;
            if file.revision.to_string() != label.revision || hash(&file.content) != label.sha256 {
                return Err("来源分类不匹配当前revision/hash".into());
            }
            labels.push(egress(&label.egress));
            sources.push(RelaySource {
                source: file.path.to_string(),
                revision: label.revision.clone(),
                sha256: label.sha256.clone(),
                retain_until_unix_seconds: label.retain_until_unix_seconds,
                model_egress: label.egress.clone(),
            });
            inputs.push(RunInput {
                path: file.path,
                revision: label.revision.clone(),
                content: file.content,
            });
        }
        let instruction = task
            .spec
            .instruction
            .clone()
            .ok_or("任务缺少显式instruction")?;
        let instruction_hash = hash(instruction.as_bytes());
        if !project
            .approved_instruction_sha256
            .contains(&instruction_hash)
        {
            return Err("本次任务指令尚未由主人按内容hash分类".into());
        }
        sources.push(RelaySource {
            source: format!("instruction:{}", task.task_id),
            revision: instruction_hash.clone(),
            sha256: instruction_hash,
            retain_until_unix_seconds: project.retain_until_unix_seconds,
            model_egress: project.instruction_egress.clone(),
        });
        sources.push(RelaySource {
            source: format!("code:{}", project.project),
            revision: project.repository_tree.clone(),
            sha256: project.repository_sha256.clone(),
            retain_until_unix_seconds: project.retain_until_unix_seconds,
            model_egress: project.repository_egress.clone(),
        });
        let now = jiff::Timestamp::now().as_second();
        let expiry = now
            .saturating_add(
                i64::try_from(profile.budget.max_seconds.saturating_add(180))
                    .map_err(|_| "预算过大")?,
            )
            .min(
                sources
                    .iter()
                    .map(|s| s.retain_until_unix_seconds)
                    .min()
                    .ok_or("快照未分类")?,
            );
        if expiry <= now {
            return Err("来源保留期限已过期".into());
        }
        let grant = RelayGrant {
            capability: JobCapability::new(
                task.task_id,
                claim.attempt.attempt_id,
                claim.attempt.fence,
                PermissionSet::new([ToolId::from_static("model.relay")]),
                expiry,
            ),
            audience: "studiolo.model-relay.v1".into(),
            snapshot_id: claim.attempt.snapshot_id.clone(),
            policy_hash: policy_hash.into(),
            role: task.spec.role.clone(),
            adapter: profile.provider.name().into(),
            provider: provider(profile.provider),
            model: profile.model.clone(),
            sources,
            limits: profile.relay_limits.clone(),
        };
        let spec = RunSpec {
            task_id: task.task_id,
            attempt_id: claim.attempt.attempt_id,
            fence: claim.attempt.fence,
            instruction,
            inputs,
            budget: profile.budget.clone(),
            execution: Some(ExecutionSpec {
                snapshot_id: claim.attempt.snapshot_id.clone(),
                policy_hash: policy_hash.into(),
                provider: profile.provider,
                model: profile.model.clone(),
                input_egress: labels,
                repository_tree: project.repository_tree.clone(),
                repository_egress: egress(&project.repository_egress),
            }),
        };
        // 发起方 `Consumer` 的身份与限定于本次尝试的 `Runner` 能力保持分离。
        if r.instance_id != claim.attempt.worker_id {
            return Err("claim不属于触发实例".into());
        }
        Ok((spec, grant, profile, project))
    }
    pub(super) fn authorization_request(
        &self,
        r: &StartRequest,
    ) -> Result<crate::board::requests::Request, String> {
        use studiolo_task_engine::TaskEngine;
        let task = self
            .authority
            .board
            .engine
            .lock()
            .map_err(|_| "黑板不可用")?
            .task(r.task_id)
            .map_err(|_| "任务不可用")?
            .ok_or("任务不存在")?;
        if !matches!(
            r.agent_type.as_str(),
            "Claude" | "Codex" | "claude" | "codex"
        ) {
            return Err("类型未授Runner启动权限".into());
        }
        Ok(crate::board::requests::Request {
            agent_type: r.agent_type.clone(),
            instance_id: r.instance_id.clone(),
            project: VaultPath::parse(task.spec.project).map_err(|_| "项目无效")?,
            operation: "runner.start".into(),
            request: serde_json::json!({}),
        })
    }
}
