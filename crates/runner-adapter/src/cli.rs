//! Provider launch profiles. No personal config or provider credentials are read.
use crate::{RunSpec, RunnerError};
use serde::{Deserialize, Serialize};

/// Supported real CLI protocol.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CliProvider {
    /// Claude Code with the Anthropic Messages protocol.
    Claude,
    /// Codex with the OpenAI Responses protocol.
    Codex,
}
/// Authoritative source classification, never inferred from Markdown metadata.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceEgress {
    /// Undeclared or prohibited: Stub only.
    Deny,
    /// No local-model adapter exists: Stub only.
    LocalOnly,
    /// Exact provider allowlist from authenticated classification.
    Providers(Vec<CliProvider>),
}
/// Binding supplied by the trusted scheduler; contains no credential.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionSpec {
    /// Immutable source snapshot identity.
    pub snapshot_id: String,
    /// Active effective-policy digest.
    pub policy_hash: String,
    /// Pinned provider selected outside the Runner.
    pub provider: CliProvider,
    /// Exact model selected outside the Runner.
    pub model: String,
    /// One authoritative classification per input, in the same order.
    pub input_egress: Vec<SourceEgress>,
    /// Scheduler-captured tree of the complete source checkout, including dirty/untracked bytes.
    pub repository_tree: String,
    /// Authenticated classification for the complete code snapshot.
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
    /// Stable adapter name.
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
