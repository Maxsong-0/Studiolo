//! 跨 owner 一致 checkpoint、独立 BackupWorker 与新目录恢复。
//! 本 crate 不持有任何 owner 的数据库连接，只消费封存票据所指的只读传输源。
//! 真实 IO 全部收敛在 `store::storage`，其余模块保持纯逻辑。

pub mod checkpoint;
pub mod recovery;
pub mod store;
pub mod worker;
