//! 真实中继的类型化策略、凭据与发布边界。
use crate::{CallId, JobCapability};
use serde::{Deserialize, Serialize};
use std::{fmt, path::Path};
use zeroize::Zeroizing;

/// 固定的上游，只能由已认证的调度器选定。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelayProvider {
    /// Anthropic Messages 接口（SSE）。
    Anthropic,
    /// OpenAI Responses 接口（SSE）。
    OpenAi,
}
impl RelayProvider {
    pub(crate) fn url(self) -> &'static str {
        match self {
            Self::Anthropic => "https://api.anthropic.com/v1/messages",
            Self::OpenAi => "https://api.openai.com/v1/responses",
        }
    }
    pub(crate) fn path(self) -> &'static str {
        match self {
            Self::Anthropic => "/v1/messages",
            Self::OpenAi => "/v1/responses",
        }
    }
}
/// 已认证的数据分级；缺失分级即视为拒绝。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelEgress {
    /// 仅限 Stub。
    #[default]
    Deny,
    /// 尚无随附的本地模型，因此同样仅限 Stub。
    LocalOnly,
    /// 精确的供应商允许清单。
    Providers(Vec<RelayProvider>),
}
impl ModelEgress {
    pub(crate) fn allows(&self, provider: RelayProvider) -> bool {
        matches!(self, Self::Providers(providers) if providers.contains(&provider))
    }
}
/// 一个已捕获来源修订版本的不可变分级。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelaySource {
    /// 来源的逻辑身份。
    pub source: String,
    /// 捕获时的修订版本。
    pub revision: String,
    /// 所捕获字节的 SHA-256。
    pub sha256: String,
    /// 来源的保留期限；原始中继输出的存活时间绝不能超过它。
    pub retain_until_unix_seconds: i64,
    /// 已认证的生效出网标签。
    #[serde(default)]
    pub model_egress: ModelEgress,
}
/// 硬上限；金额单位为百万分之一美元。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelayLimits {
    /// 请求的最大字节数；同时作为最坏情况下输入 token 数的保守上界。
    pub request_bytes: u32,
    /// SSE 响应累计的最大字节数。
    pub response_bytes: u32,
    /// 上游响应的最大 token 数。
    pub response_tokens: u32,
    /// 本次 attempt 的调用总数上限。
    pub calls: u32,
    /// 第一档提示阈值：提醒控制节奏（pacing）。
    pub pacing_tools: u32,
    /// 第二档提示阈值：提醒收尾（wrapup）。
    pub wrapup_tools: u32,
    /// 模型发出工具调用数的硬上限，在返回每个响应之前强制执行。
    pub max_tools: u32,
    /// 本次 attempt 的花费上限。
    pub budget_microusd: u64,
    /// 可信的单次请求最坏情况扣费，含工具／schema 输入开销。
    pub reservation_microusd: u64,
    /// 管理员为该具体模型钉死的输入 token 保守上限单价。
    pub input_token_microusd: u64,
    /// 管理员为该具体模型钉死的输出 token 保守上限单价。
    pub output_token_microusd: u64,
    /// 单次上游请求的最长墙钟时长。
    pub timeout_seconds: u32,
}
/// 可信调度器签发的授权，绝不从 Runner 的请求体中接收。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelayGrant {
    /// 既有的四层 capability 上限与 attempt 身份。
    pub capability: JobCapability,
    /// 与工具调用相区分的独立 audience。
    pub audience: String,
    /// 不可变的快照绑定。
    pub snapshot_id: String,
    /// 当前生效策略的摘要。
    pub policy_hash: String,
    /// 由调度器掌管的角色身份。
    pub role: String,
    /// 精确的 CLI 身份。
    pub adapter: String,
    /// 固定的上游。
    pub provider: RelayProvider,
    /// 精确的模型。
    pub model: String,
    /// 快照中的全部来源；不接受 Runner 提供的覆盖。
    pub sources: Vec<RelaySource>,
    /// 硬上限。
    pub limits: RelayLimits,
}
/// 已脱敏、不可序列化的短期 bearer 凭证；只有显式 expose 才会把它交给 attempt。
pub struct RelayTicket(Zeroizing<String>);
impl RelayTicket {
    pub(crate) fn new(value: String) -> Self {
        Self(Zeroizing::new(value))
    }
    /// 暴露短期作业凭证，用于经私有临时文件投递。
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}
impl fmt::Debug for RelayTicket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RelayTicket([redacted])")
    }
}
/// 仅 Gateway 使用的专用供应商密钥；不可序列化，也绝不出现在诊断输出中。
pub struct ProviderCredential(Zeroizing<String>);
impl ProviderCredential {
    /// 载入由进程专属密钥挂载提供的专用密钥；空值或含控制字符时拒绝。
    pub fn new(value: String) -> Result<Self, RelayError> {
        if value.is_empty() || value.chars().any(char::is_control) {
            return Err(RelayError::Credential);
        }
        Ok(Self(Zeroizing::new(value)))
    }
    pub(crate) fn expose(&self) -> &str {
        &self.0
    }
}
impl fmt::Debug for ProviderCredential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ProviderCredential([redacted])")
    }
}
/// 可信的当前策略／租约权威来源；每个边界（包括重放）都要重新评估。
pub trait RelayPolicyGuard: Send + Sync {
    /// 对照当前权威来源，复核快照分级与生效策略哈希。
    fn inspect(&self, grant: &RelayGrant, now_seconds: i64) -> Result<(), RelayError>;
}
/// Broker 发布闸门；成功返回即表示其清单标记已持久化。
pub trait RelayPublisher: Send + Sync {
    /// 经权威的产物路径为临时响应分级并封存。
    /// 必须按 `(attempt, call)` 幂等，并拒绝来源标签的变更。
    fn seal(
        &self,
        grant: &RelayGrant,
        call: &CallId,
        raw: &Path,
        sha256: &str,
    ) -> Result<(), RelayError>;
}
/// 有界的响应；真实的上游认证信息永远不会进入此对象。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RelayResponse {
    /// 协议媒体类型。
    pub content_type: &'static str,
    /// SSE 字节；只在结果持久化且清单发布之后才放出。
    pub body: Vec<u8>,
}
/// 稳定的错误类型；Display／Debug 中绝不出现上游响应体、URL 查询串、提示词或密钥。
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RelayError {
    /// capability 或权威绑定无效。
    #[error("relay authorization denied")]
    Denied,
    /// 至少有一个来源禁止使用该供应商。
    #[error("model egress denied")]
    Egress,
    /// 准入冻结生效中。
    #[error("gateway admission frozen")]
    Frozen,
    /// 预算类硬上限不满足：预留金额无法支付，或调用次数／工具调用数超限。
    #[error("relay budget exhausted")]
    Budget,
    /// 请求结构或边界与钉死的契约不符。
    #[error("relay request rejected")]
    Request,
    /// 持久化状态不可用：预留／结果存储或原始暂存区出错、空间不足。
    #[error("relay durable state unavailable")]
    Storage,
    /// 同一 call id 已绑定到不同请求；同一 attempt 重复签发授权也报此错。
    #[error("relay call id conflict")]
    Conflict,
    /// 上游可能已产生副作用；禁止自动重放。
    #[error("relay call indeterminate; explicit operator resolution required")]
    Indeterminate,
    /// 上游失败，或超出协议／大小／时间约束。
    #[error("relay upstream unavailable")]
    Upstream,
    /// 必需的权威审计追加失败。
    #[error("relay audit unavailable")]
    Audit,
    /// Broker 无法封存产物。
    #[error("relay artifact publication unavailable")]
    Publication,
    /// 专用供应商凭据不可用。
    #[error("dedicated provider credential unavailable")]
    Credential,
    /// 保留的响应已过期；绝不重复外部请求。
    #[error("relay result retention expired")]
    Expired,
}
