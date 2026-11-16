//! 实际 plane.lock 与只读 stage owner 一起存活；辅助文件不能脱离真实租约单独签封。
use super::*;
use crate::sync_plane::mirror::stage::{StageStore, checkpoint_io as io};
use std::{path::PathBuf, time::Instant};

/// 没有 save/begin/sweep 入口的平面辅助状态封存窗口。
pub struct FrozenPlaneFiles {
    path: PathBuf,
    store: PlaneStateStore,
    stage: StageStore,
    original: Option<(Vec<u8>, Stat)>,
    deadline: Instant,
}
impl FrozenPlaneFiles {
    /// 取得真正的平面排他租约；冲突、未闭合运行、待补审计及未封口文件都拒绝。
    pub fn open(path: &Path, deadline: Instant) -> Result<Self, PlaneError> {
        io::deadline(deadline)?;
        let store = PlaneStateStore::open(path)?;
        let uid = stage::caller()?;
        let original = io::read(
            store.root.as_fd(),
            PLANE_STATE_FILE,
            uid,
            PLANE_STATE_MAX_BYTES,
        )?;
        let stage = StageStore::open_existing_for_maintenance(path)?;
        let frozen = Self {
            path: path.to_path_buf(),
            store,
            stage,
            original,
            deadline,
        };
        frozen.verify()?;
        Ok(frozen)
    }
    /// 每次导出和 Bridge 门禁复验都必须经过原租约、根、恢复状态及全部保全对象。
    pub fn verify(&self) -> Result<(), PlaneError> {
        io::deadline(self.deadline)?;
        let uid = stage::caller()?;
        let root = self.store.root.as_fd();
        io::root(&self.path, root, uid)?;
        let (lease, _) = stage::open_regular_at(root, PLANE_LOCK, uid, LOCK_MAX_BYTES)
            .map_err(|_| PlaneError::Storage)?
            .ok_or(PlaneError::Storage)?;
        io::same(
            &io::metadata(lease.as_fd())?,
            &io::metadata(self.store._lease.0.as_fd())?,
        )?;
        if self.store.state.started_sequence.is_some()
            || self.store.state.pending_purge.is_some()
            || stage::entry_names(root)
                .map_err(|_| PlaneError::Storage)?
                .iter()
                .any(|name| name == PLANE_STATE_STAGE_FILE)
        {
            return Err(PlaneError::Storage);
        }
        let current = io::read(root, PLANE_STATE_FILE, uid, PLANE_STATE_MAX_BYTES)?;
        match (&self.original, current) {
            (None, None) if self.store.state == PlaneState::first_run() => {}
            (Some((expected, before)), Some((bytes, now))) if &bytes == expected => {
                io::unchanged(before, &now)?;
                if state::decode(&bytes)? != self.store.state {
                    return Err(PlaneError::Storage);
                }
            }
            _ => return Err(PlaneError::Storage),
        }
        self.stage.checkpoint_verify(self.deadline)?;
        io::deadline(self.deadline)
    }
    /// 独立空目录中生成原始恢复状态、空租约和 stage 的完整副本；首次状态的真实缺席也保留。
    pub fn export(&self, target: &Path) -> Result<(), PlaneError> {
        self.verify()?;
        let uid = stage::caller()?;
        let bytes = studiolo_sync_bridge::executor::CasLimits::internal()
            .max_stage_bytes
            .saturating_add((stage::INDEX_MAX_BYTES + PLANE_STATE_MAX_BYTES) as u64);
        let root = io::target(target, &self.path, uid, bytes)?;
        if let Some((original, _)) = &self.original {
            io::write(root.as_fd(), PLANE_STATE_FILE, original, uid)?;
        }
        io::write(root.as_fd(), PLANE_LOCK, &[], uid)?;
        self.stage.checkpoint_export(root.as_fd(), self.deadline)?;
        io::sync(root.as_fd())?;
        io::root(target, root.as_fd(), uid)?;
        let copied = Self::open(target, self.deadline)?;
        io::same(
            &io::metadata(root.as_fd())?,
            &io::metadata(copied.store.root.as_fd())?,
        )?;
        if copied.store.state != self.store.state || copied.stage.index() != self.stage.index() {
            return Err(PlaneError::Storage);
        }
        copied.verify()?;
        self.verify()
    }
}
