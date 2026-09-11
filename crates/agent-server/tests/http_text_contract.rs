//! HTTP 文本兜底的真实 loopback 契约；仅使用临时合成资料与临时签发的凭据。
#[path = "http_text/authentication.rs"]
mod authentication;
#[path = "http_text/availability.rs"]
mod availability;
#[path = "http_text/contracts.rs"]
mod contracts;
#[path = "http_text/denials.rs"]
mod denials;
#[path = "http_text/support.rs"]
mod support;
