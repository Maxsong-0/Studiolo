//! 经真实签名 Access Interface 装配的 HTTP 契约；只使用固定虚构身份。

#[path = "access/http_browser.rs"]
mod http_browser;
#[path = "access/http_contract.rs"]
mod http_tests;
#[path = "access/test_support.rs"]
mod test_support;
