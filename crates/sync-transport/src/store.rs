//! 固定 transport-private 存储：只有 `publish` 做真实 IO，其余保持纯逻辑。
//!
//! 初始化是管理员显式动作；`open_existing` 不补目录、不补空 state、不认领残留；
//! 能带着残留打开的入口只在 crate 内，由 owner 与重绑定核过权威记录之后经 `discard_residue` 丢。

pub mod paths;
pub mod publish;

use crate::owned_run::Result;

/// 完整原子替换的持久发布器。
///
/// 实现者必须保证：`replace` 要么让权威记录整体变成新版本，要么完全保持旧版本；
/// 临时文件在任何时候都不得被当作授权状态自动发布。要求 `Send`：owner 会被驱动平面
/// 放进跨线程的 `Mutex`（ADR-025 D1）。
pub trait StatePublisher: Send {
    /// 读取权威记录的完整字节。
    ///
    /// # 错误
    /// 记录缺失返回 `MissingStore`；身份或权限不符返回 `UnsafeStorage`。
    fn read(&self) -> Result<Vec<u8>>;

    /// 完整原子替换权威记录。
    ///
    /// # 错误
    /// 超过预算返回 `Budget`；底层失败返回 `Unavailable`，此时旧记录必须保持不变。
    fn replace(&mut self, bytes: &[u8]) -> Result<()>;

    /// 复验根身份、租约与固定文件；每次读取前后都要调用。
    ///
    /// # 错误
    /// 根或对象身份漂移返回 `UnsafeStorage`。
    fn check(&self) -> Result<()>;

    /// 真实运行 UID/GID。
    fn owner_identity(&self) -> (u32, u32);

    /// 打开时是否认下了上次发布留下的临时文件；只有 crate 内带残留打开的真实发布器会是 `true`。
    fn has_residue(&self) -> bool {
        false
    }

    /// 丢弃打开时认下的残留，返回是否真的丢了；没有认下的一律不动磁盘。
    /// 调用方必须先核过权威记录：残留从来不是授权状态，丢了等于在建它之前一刻被杀。
    ///
    /// # 错误
    /// 残留不见了返回 `MissingStore`；已换成别的对象返回 `UnsafeStorage`；落盘失败返回 `Unavailable`。
    fn discard_residue(&mut self) -> Result<bool> {
        Ok(false)
    }

    /// 固定源状态的独立副本；仅真实持久发布器实现，测试发布器默认不可导出。
    fn export_checkpoint(
        &self,
        _target: &std::path::Path,
        _expected: &[u8],
        _deadline: std::time::Instant,
    ) -> Result<()> {
        Err(crate::owned_run::TransportError::Unavailable)
    }
}

pub use publish::FilePublisher;
