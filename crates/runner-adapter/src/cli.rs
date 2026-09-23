//! 各供应商 CLI 的启动配置；不读取任何个人配置或供应商凭据。
use crate::{RunSpec, RunnerError};
use serde::{Deserialize, Serialize};

/// 支持的真实 CLI 协议。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CliProvider {
    /// 使用 Anthropic Messages 协议的 Claude Code。
    Claude,
    /// 使用 OpenAI Responses 协议的 Codex。
    Codex,
}
/// 权威的来源分级，绝不从 Markdown 元数据推断。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceEgress {
    /// 未声明或被禁止：仅限 Stub。
    Deny,
    /// 尚无本地模型适配器：仅限 Stub。
    LocalOnly,
    /// 来自已认证分级的精确供应商允许清单。
    Providers(Vec<CliProvider>),
}
/// 由可信调度器提供的执行绑定；不含任何凭据。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionSpec {
    /// 不可变的来源快照身份。
    pub snapshot_id: String,
    /// 当前生效策略的摘要。
    pub policy_hash: String,
    /// 在 Runner 之外选定并钉死的供应商。
    pub provider: CliProvider,
    /// 在 Runner 之外选定的精确模型。
    pub model: String,
    /// 每个输入对应一个权威分级，顺序与输入一致。
    pub input_egress: Vec<SourceEgress>,
    /// 调度器捕获的完整源码检出的 tree，含未提交与未跟踪的内容。
    pub repository_tree: String,
    /// 完整代码快照的已认证分级。
    pub repository_egress: SourceEgress,
}
impl ExecutionSpec {
    pub(crate) fn validate(
        &self,
        spec: &RunSpec,
        provider: CliProvider,
    ) -> Result<(), RunnerError> {
        if spec.instruction.len() > 1024 * 1024
            || spec
                .inputs
                .iter()
                .map(|input| input.content.len())
                .sum::<usize>()
                > 16 * 1024 * 1024
            || self.provider != provider
            || self.snapshot_id.is_empty()
            || self.policy_hash.is_empty()
            || self.model.is_empty()
            || self.model.chars().any(char::is_control)
            || self.input_egress.len() != spec.inputs.len()
            || !spec.budget.valid()
            || !matches!(&self.repository_egress, SourceEgress::Providers(allowed) if allowed.contains(&provider))
            || !matches!(self.repository_tree.len(), 40 | 64)
            || !self.repository_tree.bytes().all(|b| b.is_ascii_hexdigit())
            || self.input_egress.iter().any(|label| {
                !matches!(label,
                SourceEgress::Providers(allowed) if allowed.contains(&provider))
            })
        {
            return Err(RunnerError::InvalidExecution);
        }
        Ok(())
    }
}
impl CliProvider {
    /// 稳定的适配器名。
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
        }
    }
    pub(crate) fn args(self, model: &str) -> Vec<String> {
        let args: Vec<&str> = match self {
            Self::Claude => vec![
                "--print",
                "--bare",
                "--verbose",
                "--output-format",
                "stream-json",
                "--input-format",
                "stream-json",
                "--no-session-persistence",
                "--disable-slash-commands",
                "--strict-mcp-config",
                "--mcp-config",
                "{\"mcpServers\":{}}",
                "--no-chrome",
                "--permission-mode",
                "bypassPermissions",
                "--tools",
                "Read,Edit,Write,Glob,Grep,Bash",
                "--disallowedTools",
                "Agent,Task,WebFetch,WebSearch",
                "--model",
                model,
            ],
            Self::Codex => vec![
                "--ask-for-approval",
                "never",
                "exec",
                "--json",
                "--ephemeral",
                "--ignore-user-config",
                "--ignore-rules",
                "--skip-git-repo-check",
                "--sandbox",
                "danger-full-access",
                "--disable",
                "multi_agent",
                "-c",
                "model_provider=\"relay\"",
                "-c",
                "model_providers.relay.name=\"Studiolo\"",
                "-c",
                "model_providers.relay.base_url=\"http://127.0.0.1:8765/v1\"",
                "-c",
                "model_providers.relay.env_key=\"STUDIOLO_JOB_CAPABILITY\"",
                "-c",
                "model_providers.relay.wire_api=\"responses\"",
                "-c",
                "web_search=\"disabled\"",
                "--model",
                model,
                "-",
            ],
        };
        args.into_iter().map(String::from).collect()
    }
}
