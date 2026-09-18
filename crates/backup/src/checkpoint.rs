//! 跨 owner 一致 checkpoint：屏障、封存票据、inclusion plan 与清单。
//!
//! 全部为纯逻辑，不做任何 IO；真实落盘只发生在 [`crate::store::storage`]。

pub mod barrier;
pub mod inclusion;
pub mod manifest;
pub mod types;
