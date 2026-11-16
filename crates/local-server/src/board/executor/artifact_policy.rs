//! 从已认证的宿主项目配置及本次实际输入固定原批准；不接 RunnerReport。
use super::{ProjectConfig, hash};
use studiolo_core::{
    ArtifactProvenance, BackupScope, Classification, DataLabels, DesktopSync, ModelEgress,
    SourcePolicyRef,
};
use studiolo_gateway::RelaySource;

pub(super) fn original(
    project: &ProjectConfig,
    policy_hash: &str,
    snapshot_id: &str,
    sources: &[RelaySource],
) -> Result<ArtifactProvenance, &'static str> {
    if !project.allow_full_artifact_backup || !project.allow_artifact_desktop_sync {
        return Err("项目未明确批准产物持久化");
    }
    let retention = sources
        .iter()
        .map(|s| s.retain_until_unix_seconds)
        .min()
        .ok_or("实际输入没有留存标签")?
        .min(project.retain_until_unix_seconds);
    let source = SourcePolicyRef::new(
        format!(
            "artifact-project-{}",
            hash(project.project.as_str().as_bytes())
        ),
        policy_hash,
    )
    .map_err(|_| "实际生效策略身份无效")?;
    // 输入外发白名单不替生成正文分类；未知产物仍按 restricted/deny，后续单独批准降级。
    let labels = DataLabels::new(
        Classification::Restricted,
        BackupScope::Full,
        DesktopSync::Deny,
        ModelEgress::Deny,
    )
    .map_err(|_| "产物标签无效")?;
    let approval = serde_json::to_vec(&serde_json::json!({
        "domain":"studiolo.artifact-approval.v1", "project":project,
        "effective_policy_sha256":policy_hash, "snapshot_id":snapshot_id, "sources":sources,
    }))
    .map_err(|_| "原产物批准不可编码")?;
    ArtifactProvenance::new(source, labels, retention, hash(&approval))
        .map_err(|_| "原产物批准期限无效")
}

#[cfg(test)]
#[path = "artifact_policy_tests.rs"]
mod tests;
