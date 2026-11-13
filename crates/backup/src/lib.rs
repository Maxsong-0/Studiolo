//! 跨 owner 一致 checkpoint、独立 BackupWorker 与新目录恢复。
//! 本 crate 不持有任何 owner 的数据库连接，只消费封存票据所指的只读传输源。
//! 状态 IO 收敛在 `store::storage`，只读副本 IO 收敛在 `checkpoint::staging::io`。

pub mod checkpoint;
pub mod recovery;
pub mod store;
pub mod worker;
