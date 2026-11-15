//! 出站事件门铃（ADR-033）。
//!
//! 只往外按门铃：把权威审计账本里白名单内的动作变成瘦事件（类型、编号、时间、
//! 可选的需登录链接），按 Standard Webhooks 1.0.0 签名后发到配置的 HTTPS 地址。
//! 不带正文、路径、任务号或身份；对方要内容，得带着自己的身份经 MCP 或网页来取。
//!
//! 分工：[`classify`] 是纯映射；[`signature`] 是纯签名；真实网络 IO 只在
//! [`HttpEndpoint`]；[`run`] 把账本、状态存储、发送器串起来，时间由调用方传入。

mod endpoint;
mod event;
mod pump;
mod sign;

pub use endpoint::{
    DoorbellTransport, HttpEndpoint, LINK_VARIABLE, PostError, SECRET_VARIABLE, URL_VARIABLE,
};
pub use event::{DoorbellEvent, EventKind, classify};
pub use pump::{
    DOORBELL_AUDIT_ACTION, DOORBELL_ENABLED_ACTION, DoorbellState, MAX_EVENT_AGE_MS, RunError,
    RunOutcome, RunSummary, StateStore, run,
};
pub use sign::{Secret, SecretError, signature};
