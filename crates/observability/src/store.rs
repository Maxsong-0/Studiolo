//! 死人开关状态与出站门铃状态的私有持久存储。
//!
//! 文件 IO 只在 [`storage`] 一个文件里：死人开关用固定文件名 `deadman-state.json`，
//! 出站门铃用 `doorbell-state.json`（各加同名 `.new` 暂存件，各在自己的私有目录），
//! 其余模块保持纯逻辑。

mod doorbell_state;
mod state;
mod storage;

pub use state::STATE_MAX_BYTES;
pub use storage::{
    DOORBELL_STAGE_FILE_NAME, DOORBELL_STATE_FILE_NAME, DeadmanStore, DoorbellStore,
    STAGE_FILE_NAME, STATE_FILE_NAME,
};

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
    /// 另一个进程正独占着这个存储目录；不等待。
    #[error("存储目录正被另一个进程使用")]
    Busy,
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
