//! 三方同步规划与私有状态；纯规划器不读文件、不联网、不授予操作权限。
//! 复用 studiolo_core::VaultPath；调用者须先建立可信范围与完整观察。

pub mod executor;
pub mod sqlite;

mod evidence;
mod model;
mod planner;
mod validation;

pub use model::*;
pub use planner::{plan, retry};

#[cfg(test)]
mod tests;
