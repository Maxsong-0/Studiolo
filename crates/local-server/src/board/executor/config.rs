//! Strict, private host configuration. No long-lived credential is loaded by local-server.
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};
use studiolo_core::VaultPath;
use studiolo_gateway::{ModelEgress, RelayLimits};
use studiolo_runner_adapter::{CliProvider, RunBudget};

/// Dedicated Runner plane configuration; omitted configuration keeps all real execution closed.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RunnerConfig {
    /// Configuration version.
    pub schema: u32,
    /// Private state directory, separate from Vault and credential/key-holder roots.
    pub state_root: PathBuf,
    /// Private bounded ephemeral raw directory excluded from backup/sync.
    pub raw_root: PathBuf,
    /// Independent Gateway key-holder socket, never mounted into a Runner.
    pub upstream_socket: PathBuf,
    /// Expected distinct Gateway OS identity.
    pub upstream_uid: u32,
    /// Runtime profiles which must each pass a live probe before unfreeze.
    pub providers: Vec<ProviderConfig>,
    /// Explicit project/repository and classification allowlist.
    pub projects: Vec<ProjectConfig>,
    /// Deployment ceiling for logical Gateway tools.
    pub deployment_tools: Vec<String>,
    /// Owner-approved model relay ceiling tied to the effective policy hash.
    pub policies: Vec<PolicyConfig>,
}
/// Exact effective-policy and role ceiling selected by trusted configuration.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyConfig {
    /// Effective policy digest; any activation change requires explicit reconfiguration.
    pub hash: String,
    /// Role name from the work item.
    pub role: String,
    /// Policy tool ceiling.
    pub policy_tools: Vec<String>,
    /// Role tool ceiling.
    pub role_tools: Vec<String>,
}
/// Immutable provider/runtime identity and hard limits.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderConfig {
    /// Pinned provider.
    pub provider: CliProvider,
    /// Exact allowed model.
    pub model: String,
    /// Administrator-owned immutable minimal runtime root.
    pub runtime_root: PathBuf,
    /// Pinned native CLI digest.
    pub cli_sha256: String,
    /// Root-owned bubblewrap executable.
    pub bubblewrap: PathBuf,
    /// Root-owned systemd-run executable.
    pub systemd_run: PathBuf,
    /// Private bounded scratch filesystem at most 512 MiB.
    pub attempt_root: PathBuf,
    /// Durable retained-patch root, outside scratch and canonical Vault.
    pub retained_root: PathBuf,
    /// Work/time thresholds.
    pub budget: RunBudget,
    /// Monetary, token, request and tool ceilings.
    pub relay_limits: RelayLimits,
}
/// Authenticated classification of one exact canonical source revision.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SourceConfig {
    /// Logical path inside the explicitly allowed project.
    pub path: VaultPath,
    /// Exact classified revision.
    pub revision: String,
    /// SHA-256 of the classified bytes.
    pub sha256: String,
    /// Explicit provider permission; absent never means cloud-allowed.
    pub egress: ModelEgress,
    /// Retention ceiling.
    pub retain_until_unix_seconds: i64,
}
/// Administrator-approved project mapping; API callers never supply host paths.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectConfig {
    /// Exact project root.
    pub project: VaultPath,
    /// Trusted code checkout.
    pub repository: PathBuf,
    /// Full snapshot tree including staged/unstaged/untracked nonignored bytes.
    pub repository_tree: String,
    /// SHA-256 of the approved code snapshot manifest.
    pub repository_sha256: String,
    /// Authenticated code snapshot external-provider classification.
    pub repository_egress: ModelEgress,
    /// Owner-classified exact instruction digests; arbitrary new task prose stays denied.
    pub approved_instruction_sha256: Vec<String>,
    /// Authenticated classification for those explicitly approved instructions.
    pub instruction_egress: ModelEgress,
    /// Explicit owner approval for full generated-code artifacts in canonical backup history.
    pub allow_full_artifact_backup: bool,
    /// Explicit owner approval for generated-code artifacts in the desktop sync scope.
    pub allow_artifact_desktop_sync: bool,
    /// Code source retention bound.
    pub retain_until_unix_seconds: i64,
    /// Each canonical file available to this execution scope.
    pub sources: Vec<SourceConfig>,
}
pub(super) fn private_directory(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt;
    let meta = fs::symlink_metadata(path).map_err(|_| "Runner目录不可用")?;
    if !path.is_absolute()
        || fs::canonicalize(path).map_err(|_| "Runner目录不可用")? != path
        || !meta.is_dir()
        || meta.uid() != rustix::process::geteuid().as_raw()
        || meta.mode() & 0o777 != 0o700
    {
        return Err("Runner目录必须是本UID规范绝对路径0700".into());
    }
    Ok(())
}
impl RunnerConfig {
    pub(super) fn validate(&self) -> Result<(), String> {
        if self.schema != 1
            || self.providers.is_empty()
            || self.providers.len() > 2
            || self.projects.is_empty()
            || self.upstream_uid == 0
            || self.upstream_uid == rustix::process::geteuid().as_raw()
        {
            return Err("Runner配置版本、范围或独立Gateway身份无效".into());
        }
        private_directory(&self.state_root)?;
        private_directory(&self.raw_root)?;
        if self.state_root.starts_with(&self.raw_root)
            || self.raw_root.starts_with(&self.state_root)
        {
            return Err("Runner状态与raw目录必须分离".into());
        }
        for provider in &self.providers {
            private_directory(&provider.attempt_root)?;
            private_directory(&provider.retained_root)?;
            if !provider.budget.valid()
                || provider.cli_sha256.len() != 64
                || provider.model.is_empty()
                || u64::from(provider.relay_limits.max_tools) != provider.budget.max_tools
            {
                return Err("Runner版本或预算无效".into());
            }
        }
        Ok(())
    }
}
