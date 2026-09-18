//! `studiolo-backup` 的行为、失败与恢复契约。
//!
//! 单个集成测试目标，子模块按硬约束分组；所有假实现共用 `support`。

mod contract {
    mod barrier;
    mod inclusion;
    mod manifest;
    mod recovery_kit;
    mod recovery_point;
    mod restic;
    mod state_machine;
    mod storage;
    mod support;
    mod worker_flow;
}
