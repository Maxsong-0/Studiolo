//! `studiolo-backup` 的行为、失败与恢复契约。
//!
//! 单个集成测试目标，子模块按硬约束分组；所有假实现共用 `support`。

mod contract {
    mod audit_message;
    #[cfg(target_os = "linux")]
    mod audit_transport;
    mod barrier;
    #[cfg(unix)]
    mod final_manifest;
    mod inclusion;
    mod manifest;
    mod queue_message;
    mod recovery_kit;
    mod recovery_point;
    mod restic;
    #[cfg(unix)]
    mod staging;
    mod state_machine;
    mod storage;
    mod support;
    #[cfg(unix)]
    mod trusted_catalog;
    mod worker_flow;
}
