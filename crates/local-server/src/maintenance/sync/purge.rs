//! 清除残留的前置、确认与执行；审计恢复先于再次触碰残留树。
use super::{
    Counts, Inputs, MaintenanceError, Options, PlaneStateStore, Value, approve, guard, json,
    locate, revalidate, scope, sweep, sync_audit,
};

/// 清除 `<owner_view_root>/<folder>` 的残留：只对「上次核验排除里有、当前审批里没有、headless
/// 最近一次核对仍排除」的文件夹生效（安全顺序见 `deploy/sync-bridge/README.md` 第 8 节）。
/// 顺序：前置 → 核验趟 → 确认 → Intent 审计（账本不可用即拒绝、不删）→ 复核 → 删除趟 →
/// Outcome 审计（删除趟出错也先记 `applied=false` 再返回错误）。
pub(in crate::maintenance) fn purge_residue(
    inputs: &Inputs,
    options: &Options,
    confirm: &mut impl FnMut(&Value, &str) -> Result<(), MaintenanceError>,
    check: &impl Fn() -> Result<(), MaintenanceError>,
) -> Result<Value, MaintenanceError> {
    guard(inputs, check)?;
    let Some(folder) = options.folder.as_deref() else {
        return Err(MaintenanceError::Arguments);
    };
    let scope = scope(inputs)?;
    let approved = approve(&scope, inputs, options)?;
    let roots = approved.roots();
    let mut state =
        PlaneStateStore::open(roots.plane_root()).map_err(|_| MaintenanceError::SyncStorage)?;
    let ledger = inputs.audit.as_deref().ok_or(MaintenanceError::Audit)?;
    if let Some(report) = sync_audit::recover(ledger, &mut state)? {
        return Ok(report);
    }
    let listed = |names: &[String]| names.iter().any(|name| name == folder);
    if !listed(&state.state().verified_excluded) || listed(approved.excluded_folders()) {
        return Err(MaintenanceError::SyncResidue);
    }
    // headless 已不再排除它：残留一清、下一次 RunOnce 就可能把镜像内容当新内容上传，先恢复排除。
    if !listed(&state.state().headless_excluded) {
        return Err(MaintenanceError::SyncResidueHeadless);
    }
    guard(inputs, check)?;
    // 先整棵核验并计数：有链接、非常规对象或超预算就一个都不删。
    let (parent, name) = locate(roots.mirror_view_root(), folder)?;
    let mut planned = Counts::default();
    sweep(&parent, &name, 0, &mut planned, false)?;
    confirm(
        &json!({"folder":folder,"files":planned.files,"directories":planned.directories,
            "note":"删除只发生在镜像视图内，canonical 一侧不动；删掉的内容不进外 stage 保全。"}),
        "sync-purge-residue",
    )?;
    guard(inputs, check)?;
    let removed = sync_audit::run(ledger, &mut state, folder, &planned, |removed| {
        revalidate(&approved)?;
        guard(inputs, check)?;
        sweep(&parent, &name, 0, removed, true)
    })?;
    guard(inputs, check)?;
    Ok(json!({"command":"sync-purge-residue","folder":folder,
        "removed_files":removed.files,"removed_directories":removed.directories}))
}
