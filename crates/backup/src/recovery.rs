//! 新目录恢复的 kit 与 transcript。
//!
//! 新目录的物理重绑定由 VaultBroker 自己实现（`studiolo-vault-broker` 的
//! `restore` 模块）；这里只负责 kit、前置检查、失效事实与 transcript。

pub mod kit;
pub mod transcript;
