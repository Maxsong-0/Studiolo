//! 死人开关状态的私有持久存储。
//!
//! 本 crate 的**全部真实 IO 只在 [`storage`] 一个文件里**，固定文件名
//! `deadman-state.json`（加同名 `.new` 暂存件），其余模块保持纯逻辑。

mod state;
mod storage;

pub use state::STATE_MAX_BYTES;
pub use storage::{DeadmanStore, STAGE_FILE_NAME, STATE_FILE_NAME};

/// 持久层失败的固定分类；不回显路径或底层 errno 文本。
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum StoreError {
    /// 私有目录、权限或对象身份不满足约束。
    #[error("死人开关存储目标不安全")]
    UnsafeStorage,
    /// 存储目录或固定文件不存在。
    #[error("死人开关存储不存在")]
    Missing,
    /// 存储暂时不可用。
    #[error("死人开关存储暂时不可用")]
    Unavailable,
    /// 内容撕裂、校验和不符或不是合法文档；失败关闭，不当作空状态。
    #[error("死人开关持久状态损坏")]
    Corrupt,
    /// 版本未知，不自动迁移也不重置。
    #[error("死人开关持久状态版本未知")]
    UnsupportedSchema,
    /// 超过字节预算。
    #[error("死人开关持久状态超过字节预算")]
    TooLarge,
    /// 当前平台没有实现安全的私有本地文件约束。
    #[error("死人开关存储在当前平台不受支持")]
    UnsupportedPlatform,
}

pub use state::{decode, encode};
