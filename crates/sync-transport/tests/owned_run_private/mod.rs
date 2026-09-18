//! T-048 B3 契约测试：行为、失败、恢复三类齐备。
//!
//! 全部用例都只跑纯状态机、持久层与纯 adapter 逻辑；没有任何用例声称平台受支持。

mod adapters;
mod behaviour;
mod failure;
mod platform;
mod recovery;
mod schema;
mod store;
mod support;
