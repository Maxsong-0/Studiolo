//! 已 Issued 动作的实际执行者：canonical 侧走 `coordinator::sync_bridge` 接缝的条件写，
//! 镜像侧走 `mirror::install` 在静止窗口内原子进入视图。
//!
//! 终态只认权威回执：Broker 返回的真实 revision、镜像 renameat 后复验的身份；
//! 拿不到就是 `JobOutcome::Unknown`，绝不用退出码或「内容碰巧相同」补成成功。

use super::{PlaneError, SyncPlane, mirror::MirrorView, mirror::stage::StageStore};
use studiolo_sync_bridge::executor::{IssuedJob, JobBackend, JobOutcome, ObjectId};
use studiolo_sync_bridge::{Action, Side};
use studiolo_sync_transport::owned_run::{QuiescentMirror, SyncMode};

/// 一批执行期间的后端；持有静止票据、视图与外 stage。
pub struct PlaneJobBackend<'a> {
    plane: &'a SyncPlane,
    ticket: &'a QuiescentMirror,
    view: &'a MirrorView,
    stage: &'a mut StageStore,
    mode: SyncMode,
    now_unix: i64,
}

impl<'a> PlaneJobBackend<'a> {
    /// 组装一批的执行后端。
    #[must_use]
    pub fn new(
        plane: &'a SyncPlane,
        ticket: &'a QuiescentMirror,
        view: &'a MirrorView,
        stage: &'a mut StageStore,
        mode: SyncMode,
        now_unix: i64,
    ) -> Self {
        Self {
            plane,
            ticket,
            view,
            stage,
            mode,
            now_unix,
        }
    }

    /// canonical 侧：CreateOnly／Replace 走条件写（expected_revision＝冻结 `target_expected`），
    /// Delete 走文件操作删除；主体一律「外部：Obsidian」。
    ///
    /// # Errors
    /// 写者不可用返回 `Unavailable`。
    fn apply_canonical(
        &mut self,
        job: &IssuedJob,
        action: &Action,
    ) -> Result<JobOutcome, PlaneError> {
        // B 轮 W3 实现（接缝方法见 coordinator/sync_bridge.rs）。
        let _ = (self.plane, job, action, self.now_unix);
        Ok(JobOutcome::Unknown)
    }

    /// 镜像侧：每次先 `verify_quiescent`；MirrorRemote 下出站前把 canonical 当前版本保全到
    /// `ops/bridge-rollback/`（ADR-025 D5）；再把 stage 对象 renameat 进视图。
    ///
    /// # Errors
    /// 静止票据失效返回 `Quiescence`；stage 缺对象返回 `Storage`。
    fn apply_mirror(
        &mut self,
        job: &IssuedJob,
        action: &Action,
        staged: Option<ObjectId>,
    ) -> Result<JobOutcome, PlaneError> {
        // B 轮 W4 实现镜像安装（mirror::install）；W3 只在这里分派并做 D5 保全。
        let _ = (
            self.ticket,
            self.view,
            &mut self.stage,
            self.mode,
            job,
            action,
            staged,
        );
        Ok(JobOutcome::Unknown)
    }
}

impl JobBackend for PlaneJobBackend<'_> {
    fn apply(&mut self, job: &IssuedJob, action: &Action, staged: Option<ObjectId>) -> JobOutcome {
        let result = match action.key.target {
            Side::Canonical => self.apply_canonical(job, action),
            Side::Mirror => self.apply_mirror(job, action, staged),
        };
        // 任何失败都只能是未知：执行器不重发，下一轮按原键核对。
        result.unwrap_or(JobOutcome::Unknown)
    }
}
