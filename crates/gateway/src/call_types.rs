//! Gateway 接缝的调用值类型：幂等调用 ID、工具调用与结果、模型中继请求与响应。

use crate::GatewayError;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;
use studiolo_core::ToolId;

/// Gateway 调用的稳定幂等身份。
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct CallId(String);

impl CallId {
    /// 解析标识符：不能为空或全为空白，也不能含控制字符。
    pub fn parse(value: impl Into<String>) -> Result<Self, GatewayError> {
        let value = value.into();
        if value.trim().is_empty() || value.chars().any(char::is_control) {
            return Err(GatewayError::InvalidCallId);
        }
        Ok(Self(value))
    }

    /// 返回标识符文本。
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CallId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl<'de> Deserialize<'de> for CallId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(serde::de::Error::custom)
    }
}

/// 调用 Gateway 逻辑工具的结构化请求。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCall {
    call_id: CallId,
    tool: ToolId,
    arguments: Value,
}

impl ToolCall {
    /// 创建一次工具调用；各工具的参数校验由其适配器负责。
    #[must_use]
    pub fn new(call_id: CallId, tool: ToolId, arguments: Value) -> Self {
        Self {
            call_id,
            tool,
            arguments,
        }
    }

    /// 返回稳定的调用标识符。
    #[must_use]
    pub fn call_id(&self) -> &CallId {
        &self.call_id
    }

    /// 返回所请求的逻辑工具。
    #[must_use]
    pub fn tool(&self) -> &ToolId {
        &self.tool
    }

    /// 返回结构化的工具参数。
    #[must_use]
    pub fn arguments(&self) -> &Value {
        &self.arguments
    }
}

/// Gateway 逻辑工具返回的结构化结果。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolResult {
    pub(crate) call_id: CallId,
    pub(crate) tool: ToolId,
    pub(crate) output: Value,
}

impl ToolResult {
    /// 在 Gateway 适配器中创建结构化的工具结果。
    #[must_use]
    pub fn new(call_id: CallId, tool: ToolId, output: Value) -> Self {
        Self {
            call_id,
            tool,
            output,
        }
    }

    /// 返回稳定的调用标识符。
    #[must_use]
    pub fn call_id(&self) -> &CallId {
        &self.call_id
    }

    /// 返回逻辑工具。
    #[must_use]
    pub fn tool(&self) -> &ToolId {
        &self.tool
    }

    /// 返回内存适配器的输出。
    #[must_use]
    pub fn output(&self) -> &Value {
        &self.output
    }
}

/// Gateway 接缝中模型中继分支的请求。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelRequest {
    pub(crate) call_id: CallId,
    pub(crate) provider: String,
    pub(crate) model: String,
    prompt: String,
}

impl ModelRequest {
    /// 创建一个中继请求。
    #[must_use]
    pub fn new(
        call_id: CallId,
        provider: impl Into<String>,
        model: impl Into<String>,
        prompt: impl Into<String>,
    ) -> Self {
        Self {
            call_id,
            provider: provider.into(),
            model: model.into(),
            prompt: prompt.into(),
        }
    }

    /// 返回稳定的调用标识符。
    #[must_use]
    pub fn call_id(&self) -> &CallId {
        &self.call_id
    }

    /// 返回逻辑供应商名。
    #[must_use]
    pub fn provider(&self) -> &str {
        &self.provider
    }

    /// 返回逻辑模型名。
    #[must_use]
    pub fn model(&self) -> &str {
        &self.model
    }

    /// 返回调用方提供的中继提示词。
    #[must_use]
    pub fn prompt(&self) -> &str {
        &self.prompt
    }
}

/// Gateway 接缝中模型中继分支的结果。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelResponse {
    pub(crate) call_id: CallId,
    pub(crate) provider: String,
    pub(crate) model: String,
    pub(crate) text: String,
}

impl ModelResponse {
    /// 在 Gateway 适配器中创建结构化的模型响应。
    #[must_use]
    pub fn new(
        call_id: CallId,
        provider: impl Into<String>,
        model: impl Into<String>,
        text: impl Into<String>,
    ) -> Self {
        Self {
            call_id,
            provider: provider.into(),
            model: model.into(),
            text: text.into(),
        }
    }

    /// 返回稳定的调用标识符。
    #[must_use]
    pub fn call_id(&self) -> &CallId {
        &self.call_id
    }

    /// 返回配置的供应商。
    #[must_use]
    pub fn provider(&self) -> &str {
        &self.provider
    }

    /// 返回配置的模型。
    #[must_use]
    pub fn model(&self) -> &str {
        &self.model
    }

    /// 返回中继响应文本。
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }
}
