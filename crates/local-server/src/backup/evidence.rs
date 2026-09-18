//! 备份证据的注入口；服务端自己不去取证，只接收可信启动层给的快照。

use studiolo_backup::worker::recovery_point::RecoveryPointStatus;
use studiolo_backup::worker::state::JobState;

/// 一次备份状态快照。没有快照就只能显示未知，不能显示绿色。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BackupSnapshot {
    /// 恢复点整体判定。
    pub status: RecoveryPointStatus,
    /// 主人是否已批准备份配置。
    pub approved: bool,
    /// 最近一次作业的状态；没有作业为 `None`。
    pub last_job: Option<JobState>,
}

/// 由可信启动层实现；本 Module 不自己发起任何 IO。
pub trait BackupEvidence: Send + Sync {
    /// 取一次只读快照；取不到返回 `None`。
    fn snapshot(&self) -> Option<BackupSnapshot>;
}
