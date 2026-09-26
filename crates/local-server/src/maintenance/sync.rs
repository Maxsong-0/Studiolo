//! Bridge 平面私有存储的显式初始化与排除残留清除（T-132 P1，ADR-025 D8／D10）。两个命令都只在
//! 本机 OS 权限加既有 Broker 独占租约下运行（`studiolo-human` 在跑就拿不到租约，`Inputs::open`
//! 即拒绝——这就是「只在静止窗口内」的落实）。`sync-init` 任一项已存在即拒绝、不修复、不覆盖；
//! `sync-purge-residue` 先整棵核验再删除，遇到链接、非常规对象或超预算时一个字节都不动。本文件是
//! maintenance 里唯一做真实目录创建与删除的地方：`bridge/` 一处 `mkdir 0700`，其余只经 FD 相对调用。

use super::{MaintenanceError, inputs::Inputs, options::Options, workflow::guard};
use crate::sync_plane::{
    BRIDGE_DIR, bridge_binding, bridge_limits,
    mirror::stage::{STAGE_DIR, StageStore},
    recovery::{PLANE_STATE_FILE, PlaneStateStore},
};
use rustix::fs::{AtFlags, Dir, FileType, Mode, OFlags, open, openat, statat, unlinkat};
use serde_json::{Value, json};
use std::{ffi::CString, fs, os::fd::OwnedFd, os::unix::fs::DirBuilderExt, path::Path};
use studiolo_host_config::{
    SyncScope,
    sync_transport::{ApprovedTransport, SyncTransportApproval},
};
use studiolo_sync_bridge::sqlite::{BridgeError, BridgeState};
use studiolo_sync_transport::owned_run::{PinnedRuntime, TransportError, initialize_store};

/// 残留清除的遍历预算：目录层数上限与条目总数上限。
const MAX_DEPTH: usize = 32;
const MAX_ENTRIES: usize = 65_536;
/// 目录 FD 的固定打开方式：只读、必须是目录、不跟随链接、CLOEXEC；链接与非目录在这里就失败。
const DIRECTORY_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);

/// 显式初始化四项私有存储：传输存储 → `bridge/`＋Bridge 主库 → 外 stage → `plane-state.json`。
///
/// 任一项已存在整体拒绝：平面根下三项先探测，传输存储排在第一位自己按 `AlreadyExists` 拒绝，
/// 它拒绝时其余三项都还没动过。
pub(super) fn init(
    inputs: &Inputs,
    options: &Options,
    check: &impl Fn() -> Result<(), MaintenanceError>,
) -> Result<Value, MaintenanceError> {
    guard(inputs, check)?;
    let scope = scope(inputs)?;
    let approved = approve(&scope, inputs, options)?;
    let runtime =
        PinnedRuntime::from_approval(&approved).map_err(|_| MaintenanceError::Configuration)?;
    let plane_root = approved.roots().plane_root();
    if [BRIDGE_DIR, STAGE_DIR, PLANE_STATE_FILE]
        .iter()
        .any(|name| fs::symlink_metadata(plane_root.join(name)).is_ok())
    {
        return Err(MaintenanceError::SyncExists);
    }
    guard(inputs, check)?;
    initialize_store(&approved, &runtime)
        .map_err(|error| conflict(matches!(error, TransportError::AlreadyExists)))?;
    let bridge = plane_root.join(BRIDGE_DIR);
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&bridge)
        .map_err(|error| conflict(error.kind() == std::io::ErrorKind::AlreadyExists))?;
    revalidate(&approved)?;
    let binding = bridge_binding(&approved, inputs.roots.vault_identity());
    let opened = BridgeState::initialize(&bridge, binding, bridge_limits(&scope))
        .map_err(|error| conflict(matches!(error, BridgeError::AlreadyExists)))?;
    drop(opened);
    StageStore::initialize(plane_root).map_err(|_| MaintenanceError::SyncStorage)?;
    // plane-state.json 首建：文件不存在时打开即得默认状态（首次运行、无已核验排除项），原样落盘一次。
    let mut state = PlaneStateStore::open(plane_root).map_err(|_| MaintenanceError::SyncStorage)?;
    let initial = state.state().clone();
    state
        .save(initial)
        .map_err(|_| MaintenanceError::SyncStorage)?;
    revalidate(&approved)?;
    guard(inputs, check)?;
    Ok(json!({"command":"sync-init",
        "initialized":{"transport":true,"bridge":true,"stage":true,"plane_state":true},
        "note":"四项私有存储均为首次建立；任一项已存在时本命令整体拒绝，不修复、不覆盖。"}))
}

/// 清除 `<owner_view_root>/<folder>` 的残留：只对「上次核验排除里有、当前审批里没有」的文件夹生效。
pub(super) fn purge_residue(
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
    let state =
        PlaneStateStore::open(roots.plane_root()).map_err(|_| MaintenanceError::SyncStorage)?;
    let listed = |names: &[String]| names.iter().any(|name| name == folder);
    if !listed(&state.state().verified_excluded) || listed(approved.excluded_folders()) {
        return Err(MaintenanceError::SyncResidue);
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
    revalidate(&approved)?;
    let mut removed = Counts::default();
    sweep(&parent, &name, 0, &mut removed, true)?;
    guard(inputs, check)?;
    Ok(json!({"command":"sync-purge-residue","folder":folder,
        "removed_files":removed.files,"removed_directories":removed.directories}))
}

/// 范围批准来自维护入口已打开的同一份私有配置；缺字段即平面关闭，本命令没有可做的事。
fn scope(inputs: &Inputs) -> Result<SyncScope, MaintenanceError> {
    inputs
        .config
        .sync_scope()
        .map_err(|_| MaintenanceError::Configuration)?
        .ok_or(MaintenanceError::Configuration)
}

/// 在范围之上追加审批文档；申报的权威根与组合根一致（canonical、assets、writer-data、
/// Auth／Audit／配置目录），策略与黑板目录维护入口不知道，由启动时的同一核对兜底。
fn approve<'scope>(
    scope: &'scope SyncScope,
    inputs: &Inputs,
    options: &Options,
) -> Result<ApprovedTransport<'scope>, MaintenanceError> {
    let path = options
        .sync_transport
        .as_deref()
        .ok_or(MaintenanceError::Arguments)?;
    let approval =
        SyncTransportApproval::load(path).map_err(|_| MaintenanceError::Configuration)?;
    let config_directory = options
        .config
        .parent()
        .ok_or(MaintenanceError::Configuration)?;
    let reserved: [&Path; 6] = [
        options.vault.as_path(),
        options.assets.as_path(),
        options.writer_data.as_path(),
        inputs.config.auth_directory(),
        inputs.config.audit_directory(),
        config_directory,
    ];
    ApprovedTransport::approve(scope, approval, &reserved)
        .map_err(|_| MaintenanceError::Configuration)
}

fn revalidate(approved: &ApprovedTransport<'_>) -> Result<(), MaintenanceError> {
    approved
        .revalidate()
        .map_err(|_| MaintenanceError::Configuration)
}

fn conflict(exists: bool) -> MaintenanceError {
    if exists {
        MaintenanceError::SyncExists
    } else {
        MaintenanceError::SyncStorage
    }
}

/// 核验或删除一趟的计数；顶层文件夹本身计入目录数。
#[derive(Default)]
pub(super) struct Counts {
    pub(super) files: usize,
    pub(super) directories: usize,
}

/// 从视图根逐段打开到目标的父目录：每一段都不跟随链接、必须是目录。返回父目录 FD 与末段名。
pub(super) fn locate(
    view_root: &Path,
    folder: &str,
) -> Result<(OwnedFd, CString), MaintenanceError> {
    let mut current = open(view_root, DIRECTORY_FLAGS, Mode::empty())
        .map_err(|_| MaintenanceError::SyncResidueUnsafe)?;
    let mut segments = folder.split('/').peekable();
    while let Some(segment) = segments.next() {
        let name = CString::new(segment).map_err(|_| MaintenanceError::Arguments)?;
        if segments.peek().is_none() {
            return Ok((current, name));
        }
        current = openat(&current, &name, DIRECTORY_FLAGS, Mode::empty())
            .map_err(|_| MaintenanceError::SyncResidue)?;
    }
    Err(MaintenanceError::Arguments)
}

/// 对 `parent/name` 递归：`delete=false` 只核验并计数，`delete=true` 按同样的顺序删除。
/// 目录先清空子项再按 `REMOVEDIR` 删目录项，普通文件直接删目录项；链接与其它类型一律拒绝。
/// 删除一趟自己先整棵核验一遍：树里任何一处有链接或非常规对象，就一个字节都不删，
/// 不依赖调用方先跑核验趟。
pub(super) fn sweep(
    parent: &OwnedFd,
    name: &CString,
    depth: usize,
    counts: &mut Counts,
    delete: bool,
) -> Result<(), MaintenanceError> {
    if depth > MAX_DEPTH {
        return Err(MaintenanceError::SyncResidueUnsafe);
    }
    if delete && depth == 0 {
        sweep(parent, name, 0, &mut Counts::default(), false)?;
    }
    // 目标本身不存在＝没有残留可清；更深处消失了则是遍历期间有人在改，整体拒绝。
    let missing = if depth == 0 {
        MaintenanceError::SyncResidue
    } else {
        MaintenanceError::SyncResidueUnsafe
    };
    let stat = statat(parent, name, AtFlags::SYMLINK_NOFOLLOW).map_err(|_| missing)?;
    match FileType::from_raw_mode(stat.st_mode) {
        FileType::Directory => {
            let directory = openat(parent, name, DIRECTORY_FLAGS, Mode::empty())
                .map_err(|_| MaintenanceError::SyncResidueUnsafe)?;
            let mut children = Vec::new();
            let entries =
                Dir::read_from(&directory).map_err(|_| MaintenanceError::SyncResidueUnsafe)?;
            for entry in entries {
                let entry = entry.map_err(|_| MaintenanceError::SyncResidueUnsafe)?;
                let child = entry.file_name();
                if child.to_bytes() == b"." || child.to_bytes() == b".." {
                    continue;
                }
                children.push(child.to_owned());
                if counts.files + counts.directories + children.len() > MAX_ENTRIES {
                    return Err(MaintenanceError::SyncResidueUnsafe);
                }
            }
            for child in &children {
                sweep(&directory, child, depth + 1, counts, delete)?;
            }
            drop(directory);
            if delete {
                unlinkat(parent, name, AtFlags::REMOVEDIR)
                    .map_err(|_| MaintenanceError::SyncResidueUnsafe)?;
            }
            counts.directories += 1;
        }
        // 顶层必须是目录：排除项指的是文件夹，同名普通文件不是它的残留。
        FileType::RegularFile if depth > 0 => {
            if delete {
                unlinkat(parent, name, AtFlags::empty())
                    .map_err(|_| MaintenanceError::SyncResidueUnsafe)?;
            }
            counts.files += 1;
        }
        _ => return Err(MaintenanceError::SyncResidueUnsafe),
    }
    Ok(())
}
