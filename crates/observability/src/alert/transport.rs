//! 带外告警通道抽象。
//!
//! **秘密纪律**：收件地址与凭据只从环境变量注入，绝不写进代码、测试夹具、
//! 文档或仓库任何位置。本文件只出现环境变量的**名字**，不出现任何值。
//! 缺配置时返回 [`TransportError::NotConfigured`]，绝不静默当作发送成功。

use std::fmt;

/// 告警等级。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum AlertSeverity {
    /// 需要关注但尚未失能。
    Warning,
    /// 已经失能或证据不可信，必须立即处理。
    Critical,
}

impl AlertSeverity {
    /// 稳定机器码。
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::Warning => "warning",
            Self::Critical => "critical",
        }
    }
}

/// 幂等键。同一故障情节重试必须复用同一把键，跨重启也不变。
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct IdempotencyKey(String);

impl IdempotencyKey {
    /// 按域分隔标签与情节要素派生；同样的输入永远得到同一把键。
    #[must_use]
    pub fn derive(domain: &str, parts: &[&str]) -> Self {
        use sha2::{Digest, Sha256};
        let mut digest = Sha256::new();
        digest.update(b"studiolo.alert.idempotency.v1\0");
        digest.update((domain.len() as u64).to_be_bytes());
        digest.update(domain.as_bytes());
        for part in parts {
            digest.update((part.len() as u64).to_be_bytes());
            digest.update(part.as_bytes());
        }
        let hex = b"0123456789abcdef";
        let value = digest
            .finalize()
            .iter()
            .take(16)
            .flat_map(|byte| {
                [
                    char::from(hex[usize::from(byte >> 4)]),
                    char::from(hex[usize::from(byte & 15)]),
                ]
            })
            .collect();
        Self(value)
    }

    /// 取回键正文（32 位十六进制）。
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// 一条待发送告警。正文不含路径、主人身份或任何凭据。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AlertMessage {
    key: IdempotencyKey,
    severity: AlertSeverity,
    code: &'static str,
    summary: String,
}

/// 告警正文越界；不回显原值。
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("告警正文含有路径、控制字符或超过长度预算")]
pub struct AlertSummaryError;

impl AlertMessage {
    /// 构造告警；正文含 `/`、`\`、`@`、控制字符或超过 200 字符即拒绝。
    pub fn new(
        key: IdempotencyKey,
        severity: AlertSeverity,
        code: &'static str,
        summary: impl Into<String>,
    ) -> Result<Self, AlertSummaryError> {
        let summary = summary.into();
        if summary.is_empty()
            || summary.chars().count() > 200
            || summary
                .chars()
                .any(|value| value.is_control() || matches!(value, '/' | '\\' | '@'))
        {
            return Err(AlertSummaryError);
        }
        Ok(Self {
            key,
            severity,
            code,
            summary,
        })
    }

    /// 幂等键。
    #[must_use]
    pub fn key(&self) -> &IdempotencyKey {
        &self.key
    }

    /// 告警等级。
    #[must_use]
    pub fn severity(&self) -> AlertSeverity {
        self.severity
    }

    /// 稳定机器码。
    #[must_use]
    pub fn code(&self) -> &'static str {
        self.code
    }

    /// 中文摘要。
    #[must_use]
    pub fn summary(&self) -> &str {
        &self.summary
    }
}

/// 送达回执。只证明这一把键被该 transport 接受过一次。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeliveryReceipt {
    key: IdempotencyKey,
}

impl DeliveryReceipt {
    /// 由 transport 在真实接受后构造。
    #[must_use]
    pub fn new(key: IdempotencyKey) -> Self {
        Self { key }
    }

    /// 本次回执对应的幂等键。
    #[must_use]
    pub fn key(&self) -> &IdempotencyKey {
        &self.key
    }
}

/// 发送失败的固定分类。分类本身可以进审计与日志；原始错误正文不进。
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TransportError {
    /// 发送端未配置（缺环境变量）。这是"未配置"，不是"已发送"。
    #[error("带外告警发送端未配置")]
    NotConfigured,
    /// 发送超时；可重试。
    #[error("带外告警发送超时")]
    Timeout,
    /// 通道暂时不可用；可重试。
    #[error("带外告警通道暂时不可用")]
    Unavailable,
    /// 对端明确拒收；重试无意义。
    #[error("带外告警被对端拒收")]
    Rejected,
    /// 本地构造的消息不合法；重试无意义。
    #[error("带外告警消息不合法")]
    Malformed,
}

impl TransportError {
    /// 固定分类串，可核对、可比较，且不含任何 token 片段。
    #[must_use]
    pub fn classification(self) -> &'static str {
        match self {
            Self::NotConfigured => "not-configured",
            Self::Timeout => "timeout",
            Self::Unavailable => "unavailable",
            Self::Rejected => "rejected",
            Self::Malformed => "malformed",
        }
    }

    /// 是否值得重试。未配置、拒收与不合法都不重试。
    #[must_use]
    pub fn retryable(self) -> bool {
        matches!(self, Self::Timeout | Self::Unavailable)
    }
}

/// 带外告警通道。实现可以是真实 SMTP，也可以是测试用本地假通道。
pub trait AlertTransport: Send + Sync {
    /// 发送一条告警。同一把幂等键重复送达由调度器负责去重。
    fn deliver(&self, message: &AlertMessage) -> Result<DeliveryReceipt, TransportError>;
}

/// SMTP 端点环境变量名：完整 URL（含凭据）。**值绝不进仓库**。
pub const SMTP_URL_VARIABLE: &str = "STUDIOLO_ALERT_SMTP_URL";
/// 收件地址环境变量名。**值绝不进仓库**。
pub const SMTP_TO_VARIABLE: &str = "STUDIOLO_ALERT_TO";

/// 真实 SMTP 通道。只从环境变量取地址与凭据，自身不持有任何默认值。
///
/// 当前仓库没有装配真实 SMTP 客户端：即使环境变量齐备，本实现也返回
/// [`TransportError::Unavailable`]，**绝不冒称已送达**。发送端接上之后
/// 只需替换 [`AlertTransport::deliver`] 的实现，配置读取路径保持不变。
pub struct EnvironmentSmtpTransport {
    configured: bool,
}

impl EnvironmentSmtpTransport {
    /// 从进程环境读取配置。任一变量缺失或为空即视为未配置。
    ///
    /// 只判断"在不在"，不把值留在结构体里，因此 `Debug`／日志都不可能泄露。
    #[must_use]
    pub fn from_environment() -> Self {
        let present = |name: &str| std::env::var(name).is_ok_and(|value| !value.trim().is_empty());
        Self {
            configured: present(SMTP_URL_VARIABLE) && present(SMTP_TO_VARIABLE),
        }
    }

    /// 发送端是否已配置。
    #[must_use]
    pub fn configured(&self) -> bool {
        self.configured
    }
}

impl fmt::Debug for EnvironmentSmtpTransport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EnvironmentSmtpTransport")
            .field("configured", &self.configured)
            .finish()
    }
}

impl AlertTransport for EnvironmentSmtpTransport {
    fn deliver(&self, _message: &AlertMessage) -> Result<DeliveryReceipt, TransportError> {
        if self.configured {
            // 已配置但本轮没有真实 SMTP 客户端；如实报不可用，不伪装成功。
            Err(TransportError::Unavailable)
        } else {
            Err(TransportError::NotConfigured)
        }
    }
}
