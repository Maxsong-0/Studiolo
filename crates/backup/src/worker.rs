//! 独立 BackupWorker：状态机、固定 restic 封闭参数与恢复点年龄。
//!
//! 全部为纯逻辑；真实进程执行与落盘由调用方经 Interface 注入。

pub mod job;
pub mod recovery_point;
pub mod restic;
pub mod state;
