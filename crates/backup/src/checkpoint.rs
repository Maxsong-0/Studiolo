//! 跨 owner 一致 checkpoint：屏障、封存票据、inclusion plan 与清单。
//!
//! 判断逻辑与文件系统分离；专用 staging 只操作新建副本，绝不改写源 owner。

pub mod barrier;
pub mod inclusion;
pub mod manifest;
#[cfg(unix)]
pub mod staging;
pub mod types;
