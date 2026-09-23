//! 严格的私有宿主机配置。`local-server` 不加载任何长期凭据。
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};
use studiolo_core::VaultPath;
use studiolo_gateway::{ModelEgress, RelayLimits};
use studiolo_runner_adapter::{CliProvider, RunBudget};

/// 专用的 `Runner` 平面配置；省略该配置时，所有真实执行都保持关闭。
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RunnerConfig {
    /// 配置版本。
    pub schema: u32,
    /// 私有状态目录，与 `Vault` 及凭据／密钥持有方的根目录相互独立。
    pub state_root: PathBuf,
    /// 私有、有界的临时 `raw` 目录，不纳入备份与同步。
    pub raw_root: PathBuf,
    /// 独立的 `Gateway` 密钥持有方 `socket`，绝不挂载进 `Runner`。
    pub upstream_socket: PathBuf,
    /// 期望的、独立的 `Gateway` 操作系统身份。
    pub upstream_uid: u32,
    /// 运行时配置档；解冻前每一个都必须通过实时探针。
    pub providers: Vec<ProviderConfig>,
    /// 显式的项目／仓库与分级白名单。
    pub projects: Vec<ProjectConfig>,
    /// 逻辑 `Gateway` 工具的部署上限。
    pub deployment_tools: Vec<String>,
    /// 主人批准的模型 `relay` 上限，与生效策略哈希绑定。
    pub policies: Vec<PolicyConfig>,
}
/// 由受信任配置选定的精确生效策略与角色上限。
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyConfig {
    /// 生效策略摘要；任何激活变更都需要显式重新配置。
    pub hash: String,
    /// 取自工作项的角色名。
    pub role: String,
    /// 策略层的工具上限。
    pub policy_tools: Vec<String>,
    /// 角色层的工具上限。
    pub role_tools: Vec<String>,
}
/// 不可变的提供方／运行时身份与硬性上限。
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderConfig {
    /// 钉死的提供方。
    pub provider: CliProvider,
    /// 允许使用的精确模型。
    pub model: String,
    /// 归管理员所有、不可变的最小运行时根目录。
    pub runtime_root: PathBuf,
    /// 钉死的原生 `CLI` 摘要。
    pub cli_sha256: String,
    /// 属主为 `root` 的 `bubblewrap` 可执行文件。
    pub bubblewrap: PathBuf,
    /// 属主为 `root` 的 `systemd-run` 可执行文件。
    pub systemd_run: PathBuf,
    /// 私有、有界的 `scratch` 文件系统，至多 512 MiB。
    pub attempt_root: PathBuf,
    /// 持久的保留补丁根目录，位于 `scratch` 与 `canonical Vault` 之外。
    pub retained_root: PathBuf,
    /// 工作量／时间阈值。
    pub budget: RunBudget,
    /// 金额、`token`、请求数与工具调用数上限。
    pub relay_limits: RelayLimits,
}
/// 对某一精确 `canonical` 来源修订的已认证分级。
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SourceConfig {
    /// 位于显式允许项目内的逻辑路径。
    pub path: VaultPath,
    /// 经分级的精确修订。
    pub revision: String,
    /// 被分级字节的 `SHA-256`。
    pub sha256: String,
    /// 显式的提供方许可；缺省绝不等于允许上云。
    pub egress: ModelEgress,
    /// 留存期限上限。
    pub retain_until_unix_seconds: i64,
}
/// 管理员批准的项目映射；`API` 调用方绝不提供宿主机路径。
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectConfig {
    /// 精确的项目根。
    pub project: VaultPath,
    /// 受信任的代码检出目录。
    pub repository: PathBuf,
    /// 完整快照 `tree`，含已暂存、未暂存与未跟踪且未被忽略的字节。
    pub repository_tree: String,
    /// 已批准代码快照清单的 `SHA-256`。
    pub repository_sha256: String,
    /// 代码快照面向外部提供方的已认证分级。
    pub repository_egress: ModelEgress,
    /// 经主人分级的精确指令摘要；任意新的任务文字一律拒绝。
    pub approved_instruction_sha256: Vec<String>,
    /// 对上述显式批准指令的已认证分级。
    pub instruction_egress: ModelEgress,
    /// 主人显式批准：生成代码的完整产物可进入 `canonical` 备份历史。
    pub allow_full_artifact_backup: bool,
    /// 主人显式批准：生成代码产物可进入桌面同步范围。
    pub allow_artifact_desktop_sync: bool,
    /// 代码来源的留存期限。
    pub retain_until_unix_seconds: i64,
    /// 本执行范围可用的每个 `canonical` 文件。
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
