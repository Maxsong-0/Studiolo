//! 固定 link、一次一个运行的 SyncTransportOwner。
//! 没有受控 Linux 隔离能力时一律返回 `UnsupportedContainment`，绝不降级为 PID/PGID 观察。
//! 真实 IO 全部收敛在 `store::publish`，其余模块保持纯 DTO 与状态迁移。

pub mod linux;
pub mod owned_run;
pub mod store;
