//! 带外告警与死人开关。
//!
//! 通道是 trait；真实 SMTP 只从环境变量取地址与凭据，缺配置返回"未配置"
//! 而不是静默成功。收件地址、凭据绝不写进代码、测试夹具、文档或仓库任何位置。

mod deadman;
mod dispatch;
mod transport;

pub use deadman::{
    DEADMAN_ALERT_CODE, DeadmanDecision, DeadmanEvaluationError, DeadmanPolicy, DeadmanPolicyError,
    DeadmanReason, DeadmanState, evaluate,
};
pub use dispatch::{ALERT_AUDIT_ACTION, AlertDispatcher, DispatchOutcome, DispatcherConfigError};
pub use transport::{
    AlertMessage, AlertSeverity, AlertSummaryError, AlertTransport, DeliveryReceipt,
    EnvironmentSmtpTransport, IdempotencyKey, SMTP_TO_VARIABLE, SMTP_URL_VARIABLE, TransportError,
};
