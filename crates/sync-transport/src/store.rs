//! 固定 transport-private 存储：只有 `publish` 做真实 IO，其余保持纯逻辑。
//!
//! 初始化是管理员显式动作；`open_existing` 不补目录、不补空 state、不认领残留。

pub mod paths;
pub mod publish;

use crate::owned_run::Result;

/// 完整原子替换的持久发布器。
///
/// 实现者必须保证：`replace` 要么让权威记录整体变成新版本，要么完全保持旧版本；
/// 临时文件在任何时候都不得被当作授权状态自动发布。
pub trait StatePublisher {
    /// 读取权威记录的完整字节。
    ///
    /// # Errors
    /// 记录缺失返回 `MissingStore`；身份或权限不符返回 `UnsafeStorage`。
    fn read(&self) -> Result<Vec<u8>>;

    /// 完整原子替换权威记录。
    ///
    /// # Errors
    /// 超过预算返回 `Budget`；底层失败返回 `Unavailable`，此时旧记录必须保持不变。
    fn replace(&mut self, bytes: &[u8]) -> Result<()>;

    /// 复验根身份、租约与固定文件；每次读取前后都要调用。
    ///
    /// # Errors
    /// 根或对象身份漂移返回 `UnsafeStorage`。
    fn check(&self) -> Result<()>;

    /// 真实运行 UID/GID。
    fn owner_identity(&self) -> (u32, u32);
}

pub use publish::FilePublisher;
